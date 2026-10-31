//! `build`: drkameleon's level files plus the official list, flattened into
//! `vocab_core::CurriculumItem`s.
//!
//! Each upstream entry becomes one item: `hanzi` is the simplified form; the
//! traditional form, pinyin and meanings come from the form chosen by
//! `crate::select` (never blindly the first); `hsk_level` is the file number;
//! `curriculum_order` numbers the items by level, then frequency rank, then
//! hanzi, from 0. A word that appears in more than one level file is kept at
//! its lowest level, because the store requires unique hanzi and unique
//! orders (docs/architecture.md § 6.3).
//!
//! The build reports what it decided rather than only what it produced: how
//! many entries each selection rule decided, where the two upstreams disagree
//! about a word's level or reading, which readings it had to repair, and the
//! pinyin of a fixed list of spot-check words whose readings the form
//! selection exists to get right. A spot-check mismatch fails the build.

use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    path::Path,
};

use vocab_core::CurriculumItem;

use crate::{
    error::{Error, Result},
    official::OfficialIndex,
    output::Overrides,
    reading::{clean_pinyin, neutral_tone_distance, reading_key, respace_syllables},
    select::{select_form, Rule},
    sources::official_file,
    vocabulary::{level_file, load_level, Entry, LEVELS},
};

/// Words whose HSK reading is not their first CC-CEDICT form, or is easy to
/// get wrong, with the pinyin the bundled curriculum must have.
pub const SPOT_CHECKS: &[(&str, &str)] = &[
    ("吧", "ba"),
    ("了", "le"),
    ("的", "de"),
    ("还", "hái"),
    ("长", "cháng"),
    ("行", "xíng"),
    ("都", "dōu"),
    ("好", "hǎo"),
    ("要", "yào"),
    ("看", "kàn"),
    ("会", "huì"),
    ("中", "zhōng"),
    ("少", "shǎo"),
    ("大", "dà"),
    ("没", "méi"),
    ("地", "de"),
];

/// How many examples a report line lists.
const EXAMPLES: usize = 8;

/// Both upstreams, loaded.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// `(level, entries)` for levels 1–7, in level order.
    pub levels: Vec<(u8, Vec<Entry>)>,
    pub official: OfficialIndex,
}

/// Loads the files `fetch` wrote under `source_dir`.
pub fn load_sources(source_dir: &Path) -> Result<Sources> {
    let levels = LEVELS
        .map(|level| Ok((level, load_level(&level_file(source_dir, level))?)))
        .collect::<Result<Vec<_>>>()?;
    let official = OfficialIndex::load(&official_file(source_dir))?;
    Ok(Sources { levels, official })
}

/// One spot-check word and what the build gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotCheck {
    pub hanzi: &'static str,
    pub expected: &'static str,
    /// `None` when the word is not in the curriculum at all.
    pub actual: Option<String>,
}

impl SpotCheck {
    pub fn failed(&self) -> bool {
        self.actual
            .as_deref()
            .is_some_and(|actual| actual != self.expected)
    }
}

/// Everything `build` decided, printed as its summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildReport {
    pub official_rows: usize,
    pub total: usize,
    pub per_level: BTreeMap<u8, usize>,
    /// Entries with more than one form.
    pub multi_form: usize,
    /// How many entries each rule decided.
    pub rules: BTreeMap<Rule, usize>,
    /// `(hanzi, chosen pinyin)` of the entries rules 1b–4 decided: the ones
    /// the official reading did not confirm exactly, worth a look on review.
    pub fallback_choices: BTreeMap<Rule, Vec<(String, String)>>,
    /// Same hanzi in a later level file (or twice in one), dropped.
    pub duplicates: Vec<(String, u8)>,
    /// Entries without any form that has a reading.
    pub skipped: Vec<String>,
    pub without_official: usize,
    /// `(hanzi, file level, lowest official level)`.
    pub level_mismatches: Vec<(String, u8, u8)>,
    /// `(hanzi, chosen pinyin, official pinyin)` where they differ by more
    /// than spacing, separators and case. Single-form entries keep their only
    /// reading even then.
    pub reading_disagreements: Vec<(String, String, String)>,
    /// How many of `reading_disagreements` differ only in neutral tones
    /// (CC-CEDICT `xué sheng` vs official `xuéshēng`).
    pub neutral_tone_disagreements: usize,
    /// Readings changed by `reading::clean_pinyin`: `(raw, cleaned)`.
    pub cleaned: Vec<(String, String)>,
    /// Readings split into syllables by `reading::respace_syllables`.
    pub respaced: Vec<(String, String)>,
    /// Multi-character words whose reading is still one solid token.
    pub solid: Vec<(String, String)>,
    pub without_meaning_en: Vec<String>,
    pub meaning_de_merged: usize,
    /// Override keys that are not curriculum hanzi.
    pub unknown_overrides: Vec<String>,
    pub spot_checks: Vec<SpotCheck>,
}

impl BuildReport {
    pub fn spot_check_failures(&self) -> Vec<&SpotCheck> {
        self.spot_checks.iter().filter(|c| c.failed()).collect()
    }

    /// `Err(Error::Data)` naming every failed spot check.
    pub fn check_spot_checks(&self) -> Result<()> {
        let failures = self.spot_check_failures();
        if failures.is_empty() {
            return Ok(());
        }
        let list: Vec<String> = failures
            .iter()
            .map(|c| {
                format!(
                    "{} is {:?}, expected {:?}",
                    c.hanzi,
                    c.actual.as_deref().unwrap_or_default(),
                    c.expected
                )
            })
            .collect();
        Err(Error::Data(format!(
            "spot check failed, refusing to write the curriculum: {}",
            list.join("; ")
        )))
    }
}

/// Flattens `sources` into curriculum items in `curriculum_order` and merges
/// `de_overrides` into `meaning_de`.
pub fn build_curriculum(
    sources: &Sources,
    de_overrides: Option<&Overrides>,
) -> (Vec<CurriculumItem>, BuildReport) {
    let official = &sources.official;
    let mut report = BuildReport {
        official_rows: official.row_count(),
        ..BuildReport::default()
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut items: Vec<CurriculumItem> = Vec::new();

    let mut levels: Vec<&(u8, Vec<Entry>)> = sources.levels.iter().collect();
    levels.sort_by_key(|(level, _)| *level);
    for (level, entries) in levels {
        for entry in entries {
            let hanzi = entry.simplified.trim();
            if hanzi.is_empty() {
                report.skipped.push(entry.simplified.clone());
                continue;
            }
            if !seen.insert(hanzi.to_string()) {
                report.duplicates.push((hanzi.to_string(), *level));
                continue;
            }
            if entry.forms.len() > 1 {
                report.multi_form += 1;
            }
            let first_official = official.first_reading(hanzi);
            let Some(selection) = select_form(hanzi, &entry.forms, first_official) else {
                report.skipped.push(hanzi.to_string());
                continue;
            };
            *report.rules.entry(selection.rule).or_default() += 1;
            let form = &entry.forms[selection.index];

            let raw = form.transcriptions.pinyin.as_str();
            let mut pinyin = clean_pinyin(raw);
            if pinyin != raw.split_whitespace().collect::<Vec<_>>().join(" ") {
                report.cleaned.push((raw.to_string(), pinyin.clone()));
            }
            if let Some(spaced) = respace_syllables(&pinyin, hanzi) {
                report.respaced.push((hanzi.to_string(), spaced.clone()));
                pinyin = spaced;
            }
            if hanzi.chars().count() > 1 && pinyin.split_whitespace().count() == 1 {
                report.solid.push((hanzi.to_string(), pinyin.clone()));
            }
            if !matches!(selection.rule, Rule::SingleForm | Rule::OfficialReading) {
                report
                    .fallback_choices
                    .entry(selection.rule)
                    .or_default()
                    .push((hanzi.to_string(), pinyin.clone()));
            }

            match first_official {
                Some(reading) => {
                    if reading.level != *level {
                        report
                            .level_mismatches
                            .push((hanzi.to_string(), *level, reading.level));
                    }
                    let key = reading_key(&pinyin);
                    if !reading.alternatives().any(|alt| reading_key(alt) == key) {
                        if reading
                            .alternatives()
                            .any(|alt| neutral_tone_distance(&key, &reading_key(alt)).is_some())
                        {
                            report.neutral_tone_disagreements += 1;
                        }
                        report.reading_disagreements.push((
                            hanzi.to_string(),
                            pinyin.clone(),
                            reading.pinyin.clone(),
                        ));
                    }
                }
                None => report.without_official += 1,
            }

            let meaning_en = join_nonblank(&form.meanings, "; ");
            if meaning_en.is_none() {
                report.without_meaning_en.push(hanzi.to_string());
            }
            items.push(CurriculumItem {
                hanzi: hanzi.to_string(),
                traditional: form
                    .traditional
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string),
                hsk_level: *level,
                frequency: entry.frequency,
                curriculum_order: 0,
                pinyin,
                pos: join_nonblank(&entry.pos, ","),
                meaning_en,
                meaning_de: None,
                cloze_sentence: None,
                example_sentence_source: None,
                example_sentence_pinyin: None,
                example_sentence_target: None,
                vocab_audio_path: None,
                sentence_audio_path: None,
                image_path: None,
            });
        }
    }

    items.sort_by(|a, b| {
        (a.hsk_level, a.frequency.is_none(), a.frequency, &a.hanzi).cmp(&(
            b.hsk_level,
            b.frequency.is_none(),
            b.frequency,
            &b.hanzi,
        ))
    });
    for (order, item) in items.iter_mut().enumerate() {
        item.curriculum_order = order as i64;
        *report.per_level.entry(item.hsk_level).or_default() += 1;
    }
    report.total = items.len();

    if let Some(overrides) = de_overrides {
        let hanzi: HashSet<&str> = items.iter().map(|item| item.hanzi.as_str()).collect();
        report.unknown_overrides = overrides
            .keys()
            .filter(|key| !hanzi.contains(key.as_str()))
            .cloned()
            .collect();
        for item in &mut items {
            let german = overrides
                .get(&item.hanzi)
                .map(|text| text.trim())
                .filter(|text| !text.is_empty());
            if let Some(german) = german {
                item.meaning_de = Some(german.to_string());
                report.meaning_de_merged += 1;
            }
        }
    }

    report.spot_checks = SPOT_CHECKS
        .iter()
        .map(|&(hanzi, expected)| SpotCheck {
            hanzi,
            expected,
            actual: items
                .iter()
                .find(|item| item.hanzi == hanzi)
                .map(|item| item.pinyin.clone()),
        })
        .collect();
    (items, report)
}

/// The trimmed, non-blank parts joined by `separator`; `None` if there are
/// none.
fn join_nonblank(parts: &[String], separator: &str) -> Option<String> {
    let parts: Vec<&str> = parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(separator))
}

impl fmt::Display for BuildReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let levels: Vec<String> = self
            .per_level
            .iter()
            .map(|(level, count)| match level {
                7 => format!("7-9: {count}"),
                _ => format!("{level}: {count}"),
            })
            .collect();
        writeln!(f, "HSK curriculum build")?;
        writeln!(f, "  official list rows: {}", self.official_rows)?;
        writeln!(
            f,
            "  items: {} (by level {})",
            self.total,
            levels.join(", ")
        )?;
        writeln!(
            f,
            "  duplicates dropped: {}{}",
            self.duplicates.len(),
            examples(
                self.duplicates
                    .iter()
                    .map(|(h, l)| format!("{h} in level {l}"))
            )
        )?;
        writeln!(
            f,
            "  skipped (no usable reading): {}{}",
            self.skipped.len(),
            examples(self.skipped.iter().cloned())
        )?;
        writeln!(f, "  multi-form entries: {}", self.multi_form)?;
        writeln!(f, "  form chosen by:")?;
        let single = self.rules.get(&Rule::SingleForm).copied().unwrap_or(0);
        writeln!(f, "    {}: {single}", Rule::SingleForm.describe())?;
        for rule in Rule::CHOICES {
            let count = self.rules.get(&rule).copied().unwrap_or(0);
            let words = self
                .fallback_choices
                .get(&rule)
                .map_or(String::new(), |words| {
                    examples(words.iter().map(|(h, p)| format!("{h} {p}")))
                });
            writeln!(f, "    {}: {count}{words}", rule.describe())?;
        }
        writeln!(f, "  without an official row: {}", self.without_official)?;
        writeln!(
            f,
            "  level mismatches vs the official list: {}{}",
            self.level_mismatches.len(),
            examples(
                self.level_mismatches
                    .iter()
                    .map(|(h, ours, theirs)| format!("{h} file {ours} vs official {theirs}"))
            )
        )?;
        writeln!(
            f,
            "  chosen reading differs from the official one: {} ({} only in neutral tones){}",
            self.reading_disagreements.len(),
            self.neutral_tone_disagreements,
            examples(
                self.reading_disagreements
                    .iter()
                    .map(|(h, ours, theirs)| format!("{h} {ours} vs {theirs}"))
            )
        )?;
        writeln!(
            f,
            "  pinyin cleaned: {}{}",
            self.cleaned.len(),
            examples(
                self.cleaned
                    .iter()
                    .map(|(raw, clean)| format!("{raw} -> {clean}"))
            )
        )?;
        writeln!(
            f,
            "  pinyin re-spaced: {}{}",
            self.respaced.len(),
            examples(self.respaced.iter().map(|(h, p)| format!("{h} {p}")))
        )?;
        writeln!(
            f,
            "  multi-character words with solid pinyin: {}{}",
            self.solid.len(),
            examples(self.solid.iter().map(|(h, p)| format!("{h} {p}")))
        )?;
        writeln!(
            f,
            "  without meaning_en: {}{}",
            self.without_meaning_en.len(),
            examples(self.without_meaning_en.iter().cloned())
        )?;
        writeln!(
            f,
            "  meaning_de merged: {} (override keys not in the curriculum: {}{})",
            self.meaning_de_merged,
            self.unknown_overrides.len(),
            examples(self.unknown_overrides.iter().cloned())
        )?;
        writeln!(f, "  spot checks:")?;
        for check in &self.spot_checks {
            let verdict = match &check.actual {
                None => "absent".to_string(),
                Some(actual) if actual == check.expected => format!("{actual} ok"),
                Some(actual) => format!("{actual} MISMATCH, expected {}", check.expected),
            };
            writeln!(f, "    {} {verdict}", check.hanzi)?;
        }
        Ok(())
    }
}

/// `" (a, b, c, ...)"` for up to [`EXAMPLES`] items; empty for none.
fn examples(items: impl Iterator<Item = String>) -> String {
    let items: Vec<String> = items.collect();
    if items.is_empty() {
        return String::new();
    }
    let shown: Vec<&str> = items.iter().take(EXAMPLES).map(String::as_str).collect();
    let more = if items.len() > EXAMPLES { ", ..." } else { "" };
    format!(" ({}{more})", shown.join(", "))
}
