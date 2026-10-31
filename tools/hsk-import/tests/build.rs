//! `build` end to end over small synthetic upstream files laid out like a
//! `fetch` output: form selection, flattening, ordering, deduplication,
//! the German merge, the deterministic output and the spot-check guard.

use std::path::{Path, PathBuf};

use hsk_import::{
    cli::{run, Command},
    curriculum::{build_curriculum, load_sources, BuildReport},
    output::{curriculum_json, read_curriculum, write_overrides, Overrides},
    select::Rule,
    Error,
};
use serde_json::{json, Value};
use vocab_core::CurriculumItem;

const HEADER: &str =
    "ID,Simplified,Traditional,Pinyin,POS,Level,WebNo,WebPinyin,OCR,Variants,CEDICT";

fn form(traditional: &str, pinyin: &str, meanings: &[&str]) -> Value {
    json!({
        "traditional": traditional,
        "transcriptions": { "pinyin": pinyin, "numeric": "x" },
        "meanings": meanings,
        "classifiers": []
    })
}

fn entry(simplified: &str, frequency: Option<i64>, pos: &[&str], forms: Vec<Value>) -> Value {
    json!({
        "simplified": simplified,
        "radical": "口",
        "frequency": frequency,
        "pos": pos,
        "forms": forms
    })
}

/// A source directory with `levels` (others empty) and the official CSV
/// `rows` (without header).
fn sources(dir: &Path, levels: &[(u8, Vec<Value>)], rows: &[&str]) -> PathBuf {
    let root = dir.join("src");
    let words = root.join("complete-hsk-vocabulary/wordlists/exclusive/new");
    std::fs::create_dir_all(&words).unwrap();
    for level in 1..=7u8 {
        let entries = levels
            .iter()
            .find(|(l, _)| *l == level)
            .map_or(json!([]), |(_, entries)| json!(entries));
        std::fs::write(words.join(format!("{level}.json")), entries.to_string()).unwrap();
    }
    std::fs::create_dir_all(root.join("hsk30")).unwrap();
    let csv = format!("{HEADER}\n{}\n", rows.join("\n"));
    std::fs::write(root.join("hsk30/hsk30.csv"), csv).unwrap();
    root
}

/// 吧, 还 and 长 as upstream has them: the HSK reading is never the first
/// form. 爱 also appears again at level 2; 阿姨 sits in the 7-9 file without
/// a frequency although the official list says level 1.
fn sample(dir: &Path) -> PathBuf {
    let ba = entry(
        "吧",
        Some(58),
        &["y"],
        vec![
            form(
                "吧",
                "bā",
                &[
                    "bar (loanword) (serving drinks, or providing Internet access etc)",
                    "to puff (on a pipe etc)",
                    "(onom.) bang",
                    "abbr. for 贴吧",
                ],
            ),
            form(
                "吧",
                "ba",
                &[
                    "(modal particle indicating suggestion or surmise)",
                    "...right?",
                ],
            ),
            form("吧", "biā", &["(onom.) smack!"]),
        ],
    );
    let hai = entry(
        "还",
        Some(37),
        &["d", "v"],
        vec![
            form("還", "Huán", &["surname Huan"]),
            form("還", "hái", &["still", "yet"]),
            form("還", "huán", &["to pay back", "to return"]),
        ],
    );
    let ai = |frequency| {
        entry(
            "爱",
            Some(frequency),
            &["v"],
            vec![form("愛", "ài", &["to love; to be fond of", "affection"])],
        )
    };
    let chang = entry(
        "长",
        Some(100),
        &["a"],
        vec![
            form("長", "zhǎng", &["chief; head", "to grow"]),
            form("長", "cháng", &["length", "long"]),
        ],
    );
    let ayi = entry("阿姨", None, &[], vec![form("阿姨", "ā yí", &["aunt"])]);
    sources(
        dir,
        &[
            (1, vec![ba, hai, ai(130)]),
            (2, vec![chang, ai(5)]),
            (7, vec![ayi]),
        ],
        &[
            "L1-0005,吧,吧,ba,Aux,1,89,ba,吧,,x",
            "L1-0153,还,還,huán,V,1,3499,huán,还,,x",
            "L1-0132,还,還,hái,Adv,1,3138,hái,还,,x",
            "L1-0001,爱,愛,ài,V,1,18,ài,爱,,x",
            "L2-0715,长,長,zhǎng,V,2,10258,zhǎng,长,,x",
            "L2-0055,长,長,cháng,Adj,2,816,cháng,长,,x",
            "L1-0002,阿姨,阿姨,āyí,N,1,1,āyí,阿姨,,x",
        ],
    )
}

fn build(dir: &Path, overrides: Option<&Overrides>) -> (Vec<CurriculumItem>, BuildReport) {
    build_curriculum(&load_sources(dir).unwrap(), overrides)
}

#[test]
fn entries_flatten_into_ordered_items_from_the_official_reading() {
    let dir = tempfile::tempdir().unwrap();
    let (items, report) = build(&sample(dir.path()), None);

    let summary: Vec<(&str, u8, Option<i64>, i64, &str)> = items
        .iter()
        .map(|i| {
            (
                i.hanzi.as_str(),
                i.hsk_level,
                i.frequency,
                i.curriculum_order,
                i.pinyin.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("还", 1, Some(37), 0, "hái"),
            ("吧", 1, Some(58), 1, "ba"),
            ("爱", 1, Some(130), 2, "ài"),
            ("长", 2, Some(100), 3, "cháng"),
            ("阿姨", 7, None, 4, "ā yí"),
        ],
        "level, then frequency; readings from the official list, not forms[0]"
    );
    let ba = &items[1];
    assert_eq!(
        ba.meaning_en.as_deref(),
        Some("(modal particle indicating suggestion or surmise); ...right?")
    );
    assert_eq!(ba.traditional.as_deref(), Some("吧"));
    assert_eq!(ba.pos.as_deref(), Some("y"));
    let hai = &items[0];
    assert_eq!(hai.meaning_en.as_deref(), Some("still; yet"));
    assert_eq!(hai.traditional.as_deref(), Some("還"));
    assert_eq!(hai.pos.as_deref(), Some("d,v"));
    assert_eq!(
        items[2].meaning_en.as_deref(),
        Some("to love; to be fond of; affection")
    );
    assert_eq!(items[4].pos, None);
    assert!(items.iter().all(|i| i.meaning_de.is_none()
        && i.example_sentence_source.is_none()
        && i.vocab_audio_path.is_none()));

    assert_eq!(report.total, 5);
    assert_eq!(report.duplicates, [("爱".to_string(), 2)]);
    assert_eq!(report.multi_form, 3);
    assert_eq!(report.rules.get(&Rule::OfficialReading), Some(&3));
    assert_eq!(report.rules.get(&Rule::SingleForm), Some(&2));
    assert_eq!(report.level_mismatches, [("阿姨".to_string(), 7, 1)]);
    assert_eq!(report.official_rows, 7);
    let per_level: Vec<(u8, usize)> = report.per_level.into_iter().collect();
    assert_eq!(per_level, [(1, 3), (2, 1), (7, 1)]);
}

#[test]
fn the_summary_reports_rules_mismatches_and_spot_checks() {
    let dir = tempfile::tempdir().unwrap();
    let (_, report) = build(&sample(dir.path()), None);
    let text = report.to_string();
    for line in [
        "items: 5 (by level 1: 3, 2: 1, 7-9: 1)",
        "multi-form entries: 3",
        "rule 1 (official reading): 3",
        "level mismatches vs the official list: 1 (阿姨 file 7 vs official 1)",
        "吧 ba ok",
        "还 hái ok",
        "长 cháng ok",
        "地 absent",
    ] {
        assert!(text.contains(line), "missing {line:?} in:\n{text}");
    }
    assert!(report.check_spot_checks().is_ok());
}

#[test]
fn german_overrides_are_merged_and_unknown_keys_reported() {
    let dir = tempfile::tempdir().unwrap();
    let overrides: Overrides = [
        ("吧", " nicht wahr? "),
        ("爱", "lieben"),
        ("猫", "Katze"),
        ("还", " "),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let (items, report) = build(&sample(dir.path()), Some(&overrides));

    let german: Vec<(&str, Option<&str>)> = items
        .iter()
        .map(|i| (i.hanzi.as_str(), i.meaning_de.as_deref()))
        .collect();
    assert_eq!(
        german,
        [
            ("还", None),
            ("吧", Some("nicht wahr?")),
            ("爱", Some("lieben")),
            ("长", None),
            ("阿姨", None),
        ]
    );
    assert_eq!(report.meaning_de_merged, 2);
    assert_eq!(report.unknown_overrides, ["猫"]);
}

#[test]
fn the_output_is_deterministic_compact_and_reads_back_as_curriculum_items() {
    let dir = tempfile::tempdir().unwrap();
    let source = sample(dir.path());
    let (first, _) = build(&source, None);
    let (second, _) = build(&source, None);
    let json = curriculum_json(&first).unwrap();
    assert_eq!(json, curriculum_json(&second).unwrap());

    assert!(!json.contains("null"), "absent fields are omitted:\n{json}");
    assert_eq!(json.lines().count(), first.len() + 2, "one item per line");
    let back: Vec<CurriculumItem> = serde_json::from_str(&json).unwrap();
    assert_eq!(back, first);
    let unique: std::collections::HashSet<&str> = back.iter().map(|i| i.hanzi.as_str()).collect();
    assert_eq!(unique.len(), back.len());
}

#[test]
fn the_cli_writes_the_curriculum_and_merges_the_overrides_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = sample(dir.path());
    let overrides = dir.path().join("de.json");
    write_overrides(
        &overrides,
        &[("长".to_string(), "lang".to_string())]
            .into_iter()
            .collect(),
    )
    .unwrap();
    let out = dir.path().join("data/hsk_curriculum.json");

    run(Command::Build {
        source: source.clone(),
        out: out.clone(),
        de_overrides: Some(overrides),
    })
    .unwrap();

    let items = read_curriculum(&out).unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(items[3].meaning_de.as_deref(), Some("lang"));

    let missing = run(Command::Build {
        source,
        out: dir.path().join("other.json"),
        de_overrides: Some(dir.path().join("nope.json")),
    });
    assert!(matches!(missing, Err(Error::Data(message)) if message.contains("nope.json")));
    assert!(!dir.path().join("other.json").exists());
}

#[test]
fn a_spot_check_mismatch_fails_the_build_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    // 吧 with only its loanword reading and no official row to fix it.
    let source = sources(
        dir.path(),
        &[(
            1,
            vec![entry(
                "吧",
                Some(1),
                &["y"],
                vec![form("吧", "bā", &["bar (loanword)"])],
            )],
        )],
        &[],
    );
    let (_, report) = build(&source, None);
    assert!(matches!(
        report.check_spot_checks(),
        Err(Error::Data(message)) if message.contains("吧 is \"bā\", expected \"ba\"")
    ));
    assert!(report.to_string().contains("吧 bā MISMATCH, expected ba"));

    let out = dir.path().join("hsk_curriculum.json");
    let result = run(Command::Build {
        source,
        out: out.clone(),
        de_overrides: None,
    });
    assert!(matches!(result, Err(Error::Data(_))));
    assert!(!out.exists());
}

#[test]
fn readings_are_cleaned_and_respaced_and_entries_without_one_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let source = sources(
        dir.path(),
        &[(
            6,
            vec![
                entry(
                    "策略",
                    Some(3),
                    &["n"],
                    vec![form("策略", "cè lu:è", &["tactics"])],
                ),
                entry(
                    "电动车",
                    Some(2),
                    &["n"],
                    vec![form("電動車", "diàndòngchē", &["e-bike"])],
                ),
                entry(
                    "笑话儿",
                    Some(1),
                    &["n"],
                    vec![form("笑話兒", "xiàohuar5", &["joke"])],
                ),
                entry("空", Some(4), &[], vec![form("空", " ", &["empty"])]),
                entry(" ", Some(5), &[], vec![form("", "x", &["nothing"])]),
            ],
        )],
        &[],
    );

    let (items, report) = build(&source, None);

    let readings: Vec<(&str, &str)> = items
        .iter()
        .map(|i| (i.hanzi.as_str(), i.pinyin.as_str()))
        .collect();
    assert_eq!(
        readings,
        [
            ("笑话儿", "xiào huar"),
            ("电动车", "diàn dòng chē"),
            ("策略", "cè lüè"),
        ]
    );
    assert_eq!(report.skipped, ["空", " "]);
    assert_eq!(report.cleaned.len(), 2);
    assert_eq!(report.respaced.len(), 2);
    assert!(report.solid.is_empty());
    assert_eq!(report.without_official, 3);
}
