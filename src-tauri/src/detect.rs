//! The statistics behind the default hotkey's language detection: lingua as
//! a [`LatinGuesser`] (Milestone O; the rules are `vocab_core::detect`).
//!
//! lingua compares the text's character n-grams with a model of each
//! language. The models are compiled into the binary, so nothing is
//! downloaded and no text leaves the machine. Only the five Latin-script
//! languages the app supports are built in (the workspace pins lingua's
//! features): Chinese is decided by script before lingua is asked.
//!
//! A detector is built per call, about 40 µs; lingua keeps loaded models in
//! process-wide statics, so only the first call pays for loading them
//! (about 5 ms) and later ones take well under a millisecond. That is cheap
//! enough to run inline on the capture task. It always runs in lingua's
//! default high-accuracy mode: the low-accuracy mode was confidently wrong in
//! the calibration ("hello" read as French with 0.76).

use lingua::{Language, LanguageDetectorBuilder};
use vocab_core::{
    detect::{LanguageScore, LatinGuesser},
    language::normalize_lang,
};

/// lingua, restricted to the candidates of each call.
pub struct LinguaGuesser;

/// The canonical codes lingua is built with, and its name for each.
const LANGUAGES: [(&str, Language); 5] = [
    ("en", Language::English),
    ("de", Language::German),
    ("fr", Language::French),
    ("es", Language::Spanish),
    ("it", Language::Italian),
];

impl LatinGuesser for LinguaGuesser {
    /// Scores `text` among the `candidates` lingua knows; other codes are
    /// skipped, and fewer than two known ones give no scores (lingua's
    /// builder panics on an empty list, and one language needs no ranking).
    fn rank(&self, text: &str, candidates: &[&str]) -> Vec<LanguageScore> {
        let mut languages: Vec<Language> = Vec::new();
        for code in candidates {
            let code = normalize_lang(code);
            let known = LANGUAGES.iter().find(|(known, _)| *known == code);
            if let Some(&(_, language)) = known {
                if !languages.contains(&language) {
                    languages.push(language);
                }
            }
        }
        if languages.len() < 2 {
            return Vec::new();
        }
        LanguageDetectorBuilder::from_languages(&languages)
            .build()
            .compute_language_confidence_values(text)
            .into_iter()
            .filter_map(|(language, confidence)| {
                let (code, _) = LANGUAGES.iter().find(|(_, known)| *known == language)?;
                Some(LanguageScore {
                    lang: (*code).to_string(),
                    confidence,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use vocab_core::detect::{detect, DetectionMethod, CONFIDENT_AT};

    use super::*;

    /// lingua's scores for `text` among English and French, best first.
    fn en_fr(text: &str) -> Vec<LanguageScore> {
        let mut scores = LinguaGuesser.rank(text, &["en", "fr"]);
        scores.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
        scores
    }

    /// The calibration of lingua 1.8.0 the thresholds of `vocab_core::detect`
    /// rest on. A failure here means the models moved: re-check the
    /// thresholds before changing the expectations.
    #[test]
    fn the_calibration_words_score_as_measured() {
        for (text, lang) in [
            ("serendipity", "en"),
            ("bonjour", "fr"),
            ("toujours", "fr"),
            ("though", "en"),
        ] {
            let scores = en_fr(text);
            assert_eq!(scores.len(), 2, "{text}: {scores:?}");
            assert_eq!(scores[0].lang, lang, "{text}: {scores:?}");
            assert!(scores[0].confidence >= CONFIDENT_AT, "{text}: {scores:?}");
            let sum: f64 = scores.iter().map(|score| score.confidence).sum();
            assert!((sum - 1.0).abs() < 1e-9, "{text}: {scores:?}");
        }
        let message = en_fr("message");
        assert_eq!(message[0].lang, "fr", "{message:?}");
        assert!(message[0].confidence < CONFIDENT_AT, "{message:?}");
    }

    #[test]
    fn fewer_than_two_known_languages_give_no_scores() {
        assert!(LinguaGuesser.rank("bonjour", &[]).is_empty());
        assert!(LinguaGuesser.rank("bonjour", &["fr"]).is_empty());
        assert!(LinguaGuesser
            .rank("bonjour", &["fr", "zh-CN", "ja"])
            .is_empty());
        assert!(LinguaGuesser.rank("bonjour", &["fr", "FR"]).is_empty());

        let scores = LinguaGuesser.rank("bonjour", &["fr", "zh-CN", "EN"]);
        let mut codes: Vec<_> = scores.iter().map(|score| score.lang.as_str()).collect();
        codes.sort_unstable();
        assert_eq!(codes, ["en", "fr"], "unknown codes are ignored");
    }

    #[test]
    fn homographs_follow_the_last_approved_language_only_when_close() {
        let learned = ["en".to_string(), "fr".to_string()];
        let message = detect("message", &learned, "en", Some("en"), &LinguaGuesser);
        assert_eq!(
            (message.lang.as_str(), message.method, message.confident),
            ("en", DetectionMethod::TieBreak, false)
        );
        let important = detect("important", &learned, "en", Some("en"), &LinguaGuesser);
        assert_eq!(
            (
                important.lang.as_str(),
                important.method,
                important.confident
            ),
            ("fr", DetectionMethod::Statistical, false)
        );
    }
}
