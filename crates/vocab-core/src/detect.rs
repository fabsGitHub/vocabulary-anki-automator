//! Which of the user's languages a capture is in (Milestone O).
//!
//! With languages listed under "My languages", the Automator's default
//! hotkey no longer trusts one configured source language: it decides per
//! capture, before the pipeline runs. The script decides first, because it
//! is certain where it applies: any Han character is Chinese, as
//! [`effective_source_lang`](crate::language::effective_source_lang) has
//! always done, and kana, Hangul or Cyrillic text is in none of the
//! supported languages. Only Latin-script text among two or more learned
//! Latin-script languages needs statistics, and that part is injected as a
//! [`LatinGuesser`]: the Automator adapts lingua (`src-tauri/src/detect.rs`),
//! so this crate, and Learn with it, stay without the dependency, and the
//! rules here are tested with a fake.
//!
//! A guess below [`CONFIDENT_AT`] is still used, but marked unsure, and the
//! approval window offers the other candidates. Short words that exist in
//! several languages ("message" in English and French) are the usual case;
//! the language of the last approved card wins those when it scores within
//! [`TIE_BREAK_MARGIN`] of the top, because a learner tends to capture
//! several words of one text in a row.

use serde::{Deserialize, Serialize};

use crate::language::{is_cjk_char, normalize_lang};

/// The supported languages written in the Latin alphabet: the ones only
/// statistics can tell apart.
pub const LATIN_LANGS: &[&str] = &["en", "de", "fr", "es", "it"];
/// The top score from which a statistical guess counts as certain.
/// Calibrated against lingua 1.8.0: no wrong guess scored this high, while
/// genuine homographs peak at 0.73 and German text leaking into an en/fr
/// choice at 0.80.
pub const CONFIDENT_AT: f64 = 0.85;
/// How close the last approved language must score to the top to win an
/// unsure guess.
pub const TIE_BREAK_MARGIN: f64 = 0.25;

/// Han text is always Chinese.
const CHINESE: &str = "zh-CN";

/// The writing system most of a text's letters belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Script {
    Latin,
    /// Contains a Han character (Japanese kanji included).
    Han,
    Kana,
    Hangul,
    Cyrillic,
    /// Letters of any other script (Greek, Arabic, ...).
    Other,
    /// No letters at all: digits, punctuation, emoji.
    None,
}

/// How [`Detection::lang`] was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionMethod {
    /// By the script alone: Han text is Chinese.
    Script,
    /// The only learned language written in the text's script.
    Single,
    /// The guesser's top score.
    Statistical,
    /// The last approved language, close enough to an unsure top score.
    TieBreak,
    /// None of the learned languages fits; the configured source language.
    Fallback,
    /// Picked by the user in the approval window.
    Chosen,
}

/// One candidate's statistical score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageScore {
    pub lang: String,
    /// 0..=1; the scores of one ranking sum to 1, or are all 0.
    pub confidence: f64,
}

/// The language a capture was read in, and how sure that is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    /// Canonical code the card was built for.
    pub lang: String,
    pub method: DetectionMethod,
    pub confident: bool,
    /// Script of the text (see [`script_of`]).
    pub script: Script,
    /// What the approval window offers to switch to, in settings order;
    /// always contains `lang`.
    pub candidates: Vec<String>,
    /// Statistical scores over the Latin candidates, best first; empty
    /// unless the guesser ran.
    pub ranked: Vec<LanguageScore>,
}

/// Ranks Latin-script text among 2+ candidates (canonical codes from
/// [`LATIN_LANGS`]). Scores sum to 1 over the candidates, or are all 0 when
/// nothing fits.
pub trait LatinGuesser: Send + Sync {
    fn rank(&self, text: &str, candidates: &[&str]) -> Vec<LanguageScore>;
}

/// The script of `text`: [`Script::Han`] as soon as one Han character
/// appears, even among kana (Japanese kanji read as Chinese, as before);
/// otherwise the script most letters belong to, a tie going to the first of
/// Latin, kana, Hangul, Cyrillic, other. Digits, punctuation, emoji and
/// spaces are no letters; text without any is [`Script::None`].
pub fn script_of(text: &str) -> Script {
    if text.chars().any(is_cjk_char) {
        return Script::Han;
    }
    const ORDER: [Script; 5] = [
        Script::Latin,
        Script::Kana,
        Script::Hangul,
        Script::Cyrillic,
        Script::Other,
    ];
    let mut counts = [0usize; ORDER.len()];
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        let script = letter_script(c);
        let class = ORDER
            .iter()
            .position(|known| *known == script)
            .unwrap_or(ORDER.len() - 1);
        counts[class] += 1;
    }
    // The first maximum: `max_by_key` would return the last one.
    let mut best = 0;
    for (index, &count) in counts.iter().enumerate() {
        if count > counts[best] {
            best = index;
        }
    }
    if counts[best] == 0 {
        Script::None
    } else {
        ORDER[best]
    }
}

/// The script class of one letter (never [`Script::Han`] or
/// [`Script::None`]).
fn letter_script(c: char) -> Script {
    match c {
        'a'..='z' | 'A'..='Z' | '\u{1E00}'..='\u{1EFF}' => Script::Latin,
        '\u{00D7}' | '\u{00F7}' => Script::Other,
        '\u{00C0}'..='\u{024F}' => Script::Latin,
        '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}' => Script::Kana,
        '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}' => {
            Script::Hangul
        }
        '\u{0400}'..='\u{052F}' => Script::Cyrillic,
        _ => Script::Other,
    }
}

/// Which of `learned` (canonical codes in settings order, not empty)
/// `text` is in. `fallback` is used when none fits (the configured source
/// language), `last_confirmed` is the language of the last approved card
/// and `guesser` ranks Latin-script text among two or more Latin-script
/// learned languages; it is not asked otherwise.
pub fn detect(
    text: &str,
    learned: &[String],
    fallback: &str,
    last_confirmed: Option<&str>,
    guesser: &dyn LatinGuesser,
) -> Detection {
    let script = script_of(text);
    let learned: Vec<String> = learned.iter().map(|code| normalize_lang(code)).collect();
    let fallback = normalize_lang(fallback);
    let unsure_fallback = |candidates: Vec<String>, ranked| {
        build(
            &fallback,
            DetectionMethod::Fallback,
            false,
            script,
            candidates,
            ranked,
        )
    };
    match script {
        Script::Han => build(
            CHINESE,
            DetectionMethod::Script,
            true,
            script,
            vec![CHINESE.to_string()],
            Vec::new(),
        ),
        Script::Latin => {
            let latin: Vec<&str> = learned
                .iter()
                .map(String::as_str)
                .filter(|code| LATIN_LANGS.contains(code))
                .collect();
            let candidates: Vec<String> = latin.iter().map(|code| code.to_string()).collect();
            match latin.as_slice() {
                [] => unsure_fallback(candidates, Vec::new()),
                [only] => build(
                    only,
                    DetectionMethod::Single,
                    true,
                    script,
                    candidates,
                    Vec::new(),
                ),
                _ => {
                    let ranked = rank_among(guesser, text, &latin);
                    let Some(top) = ranked.first().filter(|top| top.confidence > 0.0) else {
                        return unsure_fallback(candidates, ranked);
                    };
                    if top.confidence >= CONFIDENT_AT {
                        let lang = top.lang.clone();
                        return build(
                            &lang,
                            DetectionMethod::Statistical,
                            true,
                            script,
                            candidates,
                            ranked,
                        );
                    }
                    let score = |lang: &str| {
                        ranked
                            .iter()
                            .find(|score| score.lang == lang)
                            .map_or(0.0, |score| score.confidence)
                    };
                    let tie_break = last_confirmed.map(normalize_lang).filter(|last| {
                        latin.contains(&last.as_str())
                            && *last != top.lang
                            && top.confidence - score(last) < TIE_BREAK_MARGIN
                    });
                    let (lang, method) = match tie_break {
                        Some(last) => (last, DetectionMethod::TieBreak),
                        None => (top.lang.clone(), DetectionMethod::Statistical),
                    };
                    build(&lang, method, false, script, candidates, ranked)
                }
            }
        }
        Script::Kana | Script::Hangul | Script::Cyrillic | Script::Other | Script::None => {
            unsure_fallback(learned, Vec::new())
        }
    }
}

/// The guesser's scores for `latin`, best first; scores for other codes are
/// dropped, and equal scores keep the settings order.
fn rank_among(guesser: &dyn LatinGuesser, text: &str, latin: &[&str]) -> Vec<LanguageScore> {
    let position = |lang: &str| latin.iter().position(|code| *code == lang);
    let mut ranked: Vec<LanguageScore> = guesser
        .rank(text, latin)
        .into_iter()
        .map(|score| LanguageScore {
            lang: normalize_lang(&score.lang),
            confidence: score.confidence,
        })
        .filter(|score| position(&score.lang).is_some())
        .collect();
    ranked.sort_by(|a, b| {
        b.confidence
            .total_cmp(&a.confidence)
            .then_with(|| position(&a.lang).cmp(&position(&b.lang)))
    });
    ranked
}

/// A [`Detection`] whose candidates are sure to contain `lang`.
fn build(
    lang: &str,
    method: DetectionMethod,
    confident: bool,
    script: Script,
    mut candidates: Vec<String>,
    ranked: Vec<LanguageScore>,
) -> Detection {
    if !candidates.iter().any(|code| code == lang) {
        candidates.push(lang.to_string());
    }
    Detection {
        lang: lang.to_string(),
        method,
        confident,
        script,
        candidates,
        ranked,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// Returns `scores` for any text and records what it was asked.
    struct FakeGuesser {
        scores: Vec<(&'static str, f64)>,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl FakeGuesser {
        fn new(scores: &[(&'static str, f64)]) -> Self {
            Self {
                scores: scores.to_vec(),
                calls: Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl LatinGuesser for FakeGuesser {
        fn rank(&self, text: &str, candidates: &[&str]) -> Vec<LanguageScore> {
            self.calls.lock().unwrap().push((
                text.to_string(),
                candidates.iter().map(|code| code.to_string()).collect(),
            ));
            self.scores
                .iter()
                .map(|&(lang, confidence)| LanguageScore {
                    lang: lang.into(),
                    confidence,
                })
                .collect()
        }
    }

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|code| code.to_string()).collect()
    }

    fn scores(list: &[(&str, f64)]) -> Vec<LanguageScore> {
        list.iter()
            .map(|&(lang, confidence)| LanguageScore {
                lang: lang.into(),
                confidence,
            })
            .collect()
    }

    fn run(text: &str, learned: &[&str], last: Option<&str>, guesser: &FakeGuesser) -> Detection {
        detect(text, &codes(learned), "en", last, guesser)
    }

    #[test]
    fn script_of_names_the_script_most_letters_use() {
        let cases = [
            ("message", Script::Latin),
            ("Ça va très bien", Script::Latin),
            ("Straße", Script::Latin),
            ("Phở", Script::Latin), // U+1EDF, Latin Extended Additional
            ("学习", Script::Han),
            ("ありがとう", Script::Kana),
            ("カタカナ", Script::Kana),
            ("ｶﾀｶﾅ", Script::Kana), // half-width katakana
            ("안녕하세요", Script::Hangul),
            ("привет", Script::Cyrillic),
            ("γεια", Script::Other),
            ("مرحبا", Script::Other),
            ("123 !!!", Script::None),
            ("🙂 2×3÷4", Script::None),
            ("", Script::None),
            ("Москва is a city", Script::Latin),
            ("Москва city", Script::Cyrillic),
            ("ab вг", Script::Latin), // a tie goes to Latin
            ("hello 你好", Script::Han),
            ("東京に行きます", Script::Han),
        ];
        for (text, expected) in cases {
            assert_eq!(script_of(text), expected, "script_of({text:?})");
        }
    }

    #[test]
    fn han_text_is_chinese_whether_or_not_chinese_is_learned() {
        for learned in [&["en", "fr", "zh-CN"][..], &["en", "fr"][..]] {
            let guesser = FakeGuesser::new(&[("en", 1.0)]);
            let found = run("学习", learned, Some("en"), &guesser);
            assert_eq!(
                found,
                Detection {
                    lang: "zh-CN".into(),
                    method: DetectionMethod::Script,
                    confident: true,
                    script: Script::Han,
                    candidates: codes(&["zh-CN"]),
                    ranked: Vec::new(),
                },
                "learned {learned:?}"
            );
            assert!(guesser.calls().is_empty());
        }
    }

    #[test]
    fn one_latin_language_needs_no_statistics() {
        let guesser = FakeGuesser::new(&[("fr", 1.0)]);
        let found = run("bonjour", &["en", "zh-CN"], None, &guesser);
        assert_eq!(
            found,
            Detection {
                lang: "en".into(),
                method: DetectionMethod::Single,
                confident: true,
                script: Script::Latin,
                candidates: codes(&["en"]),
                ranked: Vec::new(),
            }
        );
        assert!(guesser.calls().is_empty());
    }

    #[test]
    fn latin_text_without_a_latin_language_falls_back() {
        let guesser = FakeGuesser::new(&[]);
        let found = detect("bonjour", &codes(&["zh-CN"]), "FR", None, &guesser);
        assert_eq!(found.lang, "fr", "the configured fallback, normalized");
        assert_eq!(found.method, DetectionMethod::Fallback);
        assert!(!found.confident);
        assert_eq!(found.candidates, codes(&["fr"]));
        assert!(guesser.calls().is_empty());
    }

    #[test]
    fn a_high_score_is_a_confident_statistical_guess() {
        let guesser = FakeGuesser::new(&[("fr", 0.04), ("en", 0.96)]);
        let found = run("serendipity", &["fr", "zh-CN", "en"], None, &guesser);
        assert_eq!(
            found,
            Detection {
                lang: "en".into(),
                method: DetectionMethod::Statistical,
                confident: true,
                script: Script::Latin,
                candidates: codes(&["fr", "en"]),
                ranked: scores(&[("en", 0.96), ("fr", 0.04)]),
            }
        );
    }

    #[test]
    fn the_threshold_is_inclusive() {
        let at = |top: f64| {
            let guesser = FakeGuesser::new(&[("en", top), ("fr", 1.0 - top)]);
            run("word", &["en", "fr"], None, &guesser)
        };
        assert_eq!(CONFIDENT_AT, 0.85);
        let exactly = at(0.85);
        assert!(exactly.confident);
        assert_eq!(exactly.method, DetectionMethod::Statistical);
        let below = at(0.8499);
        assert!(!below.confident);
        assert_eq!(
            (below.lang.as_str(), below.method),
            ("en", DetectionMethod::Statistical)
        );
    }

    #[test]
    fn an_unsure_top_without_history_is_used_but_unsure() {
        let guesser = FakeGuesser::new(&[("en", 0.39), ("fr", 0.61)]);
        let found = run("message", &["en", "fr"], None, &guesser);
        assert_eq!(found.lang, "fr");
        assert_eq!(found.method, DetectionMethod::Statistical);
        assert!(!found.confident);
        assert_eq!(found.candidates, codes(&["en", "fr"]));
        assert_eq!(found.ranked, scores(&[("fr", 0.61), ("en", 0.39)]));
    }

    #[test]
    fn the_last_approved_language_wins_a_close_unsure_guess() {
        let guesser = FakeGuesser::new(&[("en", 0.39), ("fr", 0.61)]);
        let found = run("message", &["en", "fr"], Some("en"), &guesser);
        assert_eq!(found.lang, "en");
        assert_eq!(found.method, DetectionMethod::TieBreak);
        assert!(!found.confident);
        assert_eq!(found.ranked, scores(&[("fr", 0.61), ("en", 0.39)]));
    }

    #[test]
    fn no_tie_break_when_the_margin_is_wide_or_the_last_language_does_not_apply() {
        let wide = FakeGuesser::new(&[("en", 0.31), ("fr", 0.69)]);
        let found = run("important", &["en", "fr"], Some("en"), &wide);
        assert_eq!(
            (found.lang.as_str(), found.method),
            ("fr", DetectionMethod::Statistical)
        );

        let close = || FakeGuesser::new(&[("en", 0.39), ("fr", 0.61)]);
        for last in ["es", "zh-CN", "de", "fr"] {
            let found = run("message", &["en", "fr"], Some(last), &close());
            assert_eq!(
                (found.lang.as_str(), found.method),
                ("fr", DetectionMethod::Statistical),
                "last {last}"
            );
        }
    }

    #[test]
    fn nothing_fitting_falls_back_and_keeps_the_scores() {
        let zeros = FakeGuesser::new(&[("en", 0.0), ("fr", 0.0)]);
        let found = run("xqzt", &["en", "fr"], Some("fr"), &zeros);
        assert_eq!(found.lang, "en");
        assert_eq!(found.method, DetectionMethod::Fallback);
        assert!(!found.confident);
        assert_eq!(found.ranked, scores(&[("en", 0.0), ("fr", 0.0)]));

        let nothing = FakeGuesser::new(&[]);
        let found = detect("xqzt", &codes(&["fr", "it"]), "en", None, &nothing);
        assert_eq!(found.lang, "en");
        assert_eq!(found.method, DetectionMethod::Fallback);
        assert!(found.ranked.is_empty());
        assert_eq!(
            found.candidates,
            codes(&["fr", "it", "en"]),
            "the fallback is offered too"
        );
    }

    #[test]
    fn foreign_scripts_fall_back_and_offer_every_learned_language() {
        for text in ["ありがとう", "안녕하세요", "привет", "γεια", "123 !!!"] {
            let guesser = FakeGuesser::new(&[("fr", 1.0)]);
            let found = detect(text, &codes(&["fr", "zh-CN"]), "en", Some("fr"), &guesser);
            assert_eq!(found.lang, "en", "{text}");
            assert_eq!(found.method, DetectionMethod::Fallback, "{text}");
            assert!(!found.confident, "{text}");
            assert_eq!(found.candidates, codes(&["fr", "zh-CN", "en"]), "{text}");
            assert!(found.ranked.is_empty(), "{text}");
            assert!(guesser.calls().is_empty(), "{text}");
        }
        let guesser = FakeGuesser::new(&[]);
        let found = detect("привет", &codes(&["en", "fr"]), "en", None, &guesser);
        assert_eq!(found.script, Script::Cyrillic);
        assert_eq!(
            found.candidates,
            codes(&["en", "fr"]),
            "a learned fallback is not added twice"
        );
    }

    #[test]
    fn the_guesser_only_ever_sees_two_or_more_latin_languages_in_settings_order() {
        let guesser = FakeGuesser::new(&[("it", 0.9), ("en", 0.1)]);
        run("ciao", &["zh-CN", "it", "en"], None, &guesser);
        run("ciao", &["zh-CN", "it"], None, &guesser);
        run("学习", &["it", "en"], None, &guesser);
        run("привет", &["it", "en"], None, &guesser);
        assert_eq!(
            guesser.calls(),
            [("ciao".to_string(), codes(&["it", "en"]))]
        );
    }

    #[test]
    fn scores_for_other_codes_are_dropped_and_ties_keep_the_settings_order() {
        let guesser = FakeGuesser::new(&[("de", 0.9), ("FR", 0.05), ("en", 0.05)]);
        let found = run("word", &["en", "fr"], None, &guesser);
        assert_eq!(found.ranked, scores(&[("en", 0.05), ("fr", 0.05)]));
        assert_eq!(
            (found.lang.as_str(), found.method),
            ("en", DetectionMethod::Statistical)
        );
    }

    #[test]
    fn the_last_language_is_normalized() {
        let guesser = FakeGuesser::new(&[("en", 0.39), ("fr", 0.61)]);
        let found = run("message", &["en", "fr"], Some(" EN "), &guesser);
        assert_eq!(
            (found.lang.as_str(), found.method),
            ("en", DetectionMethod::TieBreak)
        );
    }

    #[test]
    fn detection_serializes_in_snake_case() {
        let detection = Detection {
            lang: "en".into(),
            method: DetectionMethod::TieBreak,
            confident: false,
            script: Script::Latin,
            candidates: codes(&["en", "fr"]),
            ranked: scores(&[("fr", 0.61), ("en", 0.39)]),
        };
        let json = serde_json::to_value(&detection).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "lang": "en",
                "method": "tie_break",
                "confident": false,
                "script": "latin",
                "candidates": ["en", "fr"],
                "ranked": [
                    { "lang": "fr", "confidence": 0.61 },
                    { "lang": "en", "confidence": 0.39 }
                ]
            })
        );
        assert_eq!(
            serde_json::from_value::<Detection>(json).unwrap(),
            detection
        );
        for (method, name) in [
            (DetectionMethod::Script, "script"),
            (DetectionMethod::Single, "single"),
            (DetectionMethod::Statistical, "statistical"),
            (DetectionMethod::Fallback, "fallback"),
            (DetectionMethod::Chosen, "chosen"),
        ] {
            assert_eq!(serde_json::to_value(method).unwrap(), name);
        }
        for (script, name) in [
            (Script::Han, "han"),
            (Script::Kana, "kana"),
            (Script::Hangul, "hangul"),
            (Script::Cyrillic, "cyrillic"),
            (Script::Other, "other"),
            (Script::None, "none"),
        ] {
            assert_eq!(serde_json::to_value(script).unwrap(), name);
        }
    }
}
