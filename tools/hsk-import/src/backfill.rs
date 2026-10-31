//! `backfill-de`: German meanings, machine-translated from English with
//! vocab-core's `MyMemoryProvider`.
//!
//! Neither upstream has German, and the plan wants `meaning_de` for the
//! whole curriculum. MyMemory's free quota (a few thousand characters a day
//! anonymously, more with a contact e-mail) is far smaller than ~11,000
//! meanings, so the backfill is built to run many times:
//!
//! - it translates only the first sense of each meaning (`to love` for
//!   `to love; to be fond of; to like; affection`), which is what a card
//!   shows first and costs a fraction of the quota;
//! - it walks the curriculum in `curriculum_order`, so whatever has been
//!   translated covers the words a learner meets first;
//! - it writes the overrides file after every item, so an interruption loses
//!   at most the request in flight, and a rerun skips everything already in
//!   the file;
//! - it stops cleanly when MyMemory reports the quota used up.
//!
//! The results go to an overrides file, not into the curriculum itself, so
//! `build` can regenerate the curriculum from the pinned sources and merge
//! the German back in.

use std::{fmt, path::Path, time::Duration};

use vocab_core::{providers::TranslationProvider, CurriculumItem, ProviderError};

use crate::{
    error::Result,
    output::{read_overrides, write_overrides},
};

/// Limits for one run.
#[derive(Debug, Clone, Default)]
pub struct BackfillOptions {
    /// At most this many translation requests.
    pub limit: Option<usize>,
    /// Pause between requests, to be gentle with a free service.
    pub delay: Duration,
}

/// Why a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// Every item without German was tried.
    Completed,
    /// `--limit` requests were sent.
    LimitReached,
    /// MyMemory's quota is used up; its message says when it resets.
    QuotaExceeded(String),
    /// A network or HTTP failure that would fail every further request too.
    Failed(ProviderError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillReport {
    /// `(hanzi, English sent, German received)`, in request order.
    pub translated: Vec<(String, String, String)>,
    /// Items MyMemory had no usable answer for; retried by the next run.
    pub skipped: Vec<(String, ProviderError)>,
    /// Items that already had German before this run.
    pub already_done: usize,
    /// Items without an English meaning to translate.
    pub untranslatable: usize,
    /// Items still without German after this run.
    pub remaining: usize,
    pub stop: Stop,
}

/// The first sense of a `meaning_en`: the first non-blank text between `;`s
/// that are not inside brackets, trimmed. `None` for a blank meaning.
pub fn first_sense(meaning_en: &str) -> Option<&str> {
    let mut depth = 0usize;
    let mut start = 0;
    for (index, c) in meaning_en.char_indices() {
        match c {
            '(' | '（' | '[' => depth += 1,
            ')' | '）' | ']' => depth = depth.saturating_sub(1),
            ';' | '；' if depth == 0 => {
                let sense = meaning_en[start..index].trim();
                if !sense.is_empty() {
                    return Some(sense);
                }
                start = index + c.len_utf8();
            }
            _ => {}
        }
    }
    let sense = meaning_en[start..].trim();
    (!sense.is_empty()).then_some(sense)
}

fn has_german(item: &CurriculumItem) -> bool {
    item.meaning_de
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty())
}

/// Translates the first sense of every item without German (in the
/// curriculum or in the overrides file at `overrides_path`) from English to
/// German, in `curriculum_order`, writing the overrides file after every
/// translation. Provider failures end the run (`Stop`) rather than failing
/// it; only I/O on the overrides file is an `Err`.
pub async fn backfill_de(
    provider: &dyn TranslationProvider,
    curriculum: &[CurriculumItem],
    overrides_path: &Path,
    options: &BackfillOptions,
) -> Result<BackfillReport> {
    let mut overrides = read_overrides(overrides_path)?.unwrap_or_default();
    let mut items: Vec<&CurriculumItem> = curriculum.iter().collect();
    items.sort_by_key(|item| item.curriculum_order);

    let mut already_done = 0;
    let mut untranslatable = 0;
    let mut pending: Vec<(&CurriculumItem, &str)> = Vec::new();
    for item in items {
        if has_german(item) || overrides.contains_key(&item.hanzi) {
            already_done += 1;
            continue;
        }
        match item.meaning_en.as_deref().and_then(first_sense) {
            Some(sense) => pending.push((item, sense)),
            None => untranslatable += 1,
        }
    }

    let mut translated = Vec::new();
    let mut skipped = Vec::new();
    let mut stop = Stop::Completed;
    for (requests, (item, sense)) in pending.iter().enumerate() {
        if options.limit.is_some_and(|limit| requests >= limit) {
            stop = Stop::LimitReached;
            break;
        }
        if requests > 0 && !options.delay.is_zero() {
            tokio::time::sleep(options.delay).await;
        }
        match provider.translate(sense, "en", "de").await {
            Ok(german) => {
                let german = german.trim().to_string();
                overrides.insert(item.hanzi.clone(), german.clone());
                write_overrides(overrides_path, &overrides)?;
                translated.push((item.hanzi.clone(), sense.to_string(), german));
            }
            Err(ProviderError::QuotaExceeded { message, .. }) => {
                stop = Stop::QuotaExceeded(message);
                break;
            }
            Err(
                error @ (ProviderError::NotFound { .. }
                | ProviderError::Malformed { .. }
                | ProviderError::Unsupported { .. }),
            ) => skipped.push((item.hanzi.clone(), error)),
            Err(error) => {
                stop = Stop::Failed(error);
                break;
            }
        }
    }
    let remaining = pending.len() - translated.len();
    Ok(BackfillReport {
        translated,
        skipped,
        already_done,
        untranslatable,
        remaining,
        stop,
    })
}

impl fmt::Display for BackfillReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "German backfill (MyMemory, en -> de, first sense only)")?;
        for (hanzi, english, german) in &self.translated {
            writeln!(f, "  {hanzi}: {english} -> {german}")?;
        }
        for (hanzi, error) in &self.skipped {
            writeln!(f, "  {hanzi}: skipped ({error})")?;
        }
        writeln!(
            f,
            "  translated now: {}, skipped: {}, already had German: {}, \
             no English meaning: {}, still without German: {}",
            self.translated.len(),
            self.skipped.len(),
            self.already_done,
            self.untranslatable,
            self.remaining
        )?;
        let stop = match &self.stop {
            Stop::Completed => "every item without German was tried".to_string(),
            Stop::LimitReached => "request limit reached; run again to continue".to_string(),
            Stop::QuotaExceeded(message) => {
                format!("MyMemory quota used up ({message}); run again once it resets")
            }
            Stop::Failed(error) => format!("stopped by a provider failure: {error}"),
        };
        writeln!(f, "  stopped: {stop}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sense_is_the_text_before_the_first_top_level_semicolon() {
        let cases = [
            ("to love; to be fond of; to like", Some("to love")),
            ("still", Some("still")),
            (
                "(modal particle indicating suggestion or surmise); ...right?",
                Some("(modal particle indicating suggestion or surmise)"),
            ),
            (
                "(of a place; of a time) quiet; peaceful",
                Some("(of a place; of a time) quiet"),
            ),
            (
                "[Buddhism; Taoism] temple; shrine",
                Some("[Buddhism; Taoism] temple"),
            ),
            ("  ; to be", Some("to be")),
            ("食物；吃", Some("食物")),
            ("(unbalanced; still one", Some("(unbalanced; still one")),
            ("   ", None),
            (" ; ", None),
            ("", None),
        ];
        for (meaning, expected) in cases {
            assert_eq!(first_sense(meaning), expected, "{meaning:?}");
        }
    }
}
