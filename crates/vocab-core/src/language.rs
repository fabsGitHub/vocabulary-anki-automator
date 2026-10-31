//! Language-code normalization and script detection (Milestones C, J4).
//!
//! Language codes reach the core from several places that never agreed on a
//! spelling: the legacy config stored DeepL-style `EN`/`DE`, browsers and OS
//! locales say `zh_CN` or `zh-Hans`, MyMemory documents `ZH-CN`. Everything
//! downstream (config, `Card`, provider calls, the database, the provider
//! registry) compares codes as plain strings, so they are normalized once, here,
//! to the canonical set in [`SUPPORTED_LANGS`].
//!
//! Script detection exists because the configured source language lags behind
//! what the user actually copied: a user set up for English who captures
//! `学习` must still be routed to the Chinese provider set (Milestone J4), never
//! to the English-only Datamuse and dictionaryapi.dev providers. Only Han
//! ideographs are sniffed; Japanese kanji therefore also read as Chinese, which
//! is acceptable because Japanese is not a supported language.

/// Canonical language codes: lowercase ISO 639-1, except `zh-CN`, whose region
/// is uppercase as in BCP 47, the spelling MyMemory and Google TTS expect.
pub const SUPPORTED_LANGS: &[&str] = &["en", "de", "fr", "es", "it", "zh-CN"];

/// The canonical code for Simplified Chinese.
const CHINESE: &str = "zh-CN";

/// Spellings of Simplified Chinese seen in configs and locales, after
/// lowercasing and turning `_` into `-`.
const CHINESE_ALIASES: &[&str] = &["zh", "zh-cn", "zh-hans", "zh-hans-cn"];

/// Maps a language code to its canonical form: `"EN"` -> `"en"`,
/// `"zh"`/`"ZH"`/`"zh-cn"`/`"zh_CN"`/`"zh-Hans"` -> `"zh-CN"`. Codes outside
/// [`SUPPORTED_LANGS`] come back trimmed and lowercased, so they still compare
/// consistently (and fall through to the registry's default provider set).
pub fn normalize_lang(code: &str) -> String {
    let lower = code.trim().to_lowercase();
    let unified = lower.replace('_', "-");
    if CHINESE_ALIASES.contains(&unified.as_str()) {
        return CHINESE.to_string();
    }
    SUPPORTED_LANGS
        .iter()
        .find(|supported| supported.eq_ignore_ascii_case(&unified))
        .map_or(lower, |supported| (*supported).to_string())
}

/// Whether `code` denotes (Simplified) Chinese in any accepted spelling.
pub fn is_chinese(code: &str) -> bool {
    normalize_lang(code) == CHINESE
}

/// CJK Unified Ideographs (U+4E00..=U+9FFF) and Extension A (U+3400..=U+4DBF):
/// the blocks that hold every HSK character plus the rarer ones a capture may
/// contain.
pub fn is_cjk_char(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}

/// Whether `text` contains at least one [CJK ideograph](is_cjk_char).
pub fn contains_cjk(text: &str) -> bool {
    text.chars().any(is_cjk_char)
}

/// The [CJK ideographs](is_cjk_char) of `text`, in order, duplicates kept.
pub fn cjk_chars(text: &str) -> Vec<char> {
    text.chars().filter(|c| is_cjk_char(*c)).collect()
}

/// The language a capture is processed in: `"zh-CN"` whenever `text` contains a
/// CJK ideograph, whatever was configured; otherwise the normalized
/// `configured` code.
pub fn effective_source_lang(configured: &str, text: &str) -> String {
    if contains_cjk(text) {
        CHINESE.to_string()
    } else {
        normalize_lang(configured)
    }
}

/// Whether `text` is a multi-word phrase, i.e. still contains whitespace (any
/// Unicode whitespace, including tabs and newlines) once trimmed. Phrases skip
/// the single-word rarity check but are still translated (Milestone H).
pub fn is_phrase(text: &str) -> bool {
    text.trim().chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_lang_maps_every_known_spelling() {
        let cases = [
            ("en", "en"),
            ("EN", "en"),
            (" En ", "en"),
            ("DE", "de"),
            ("fr", "fr"),
            ("ES", "es"),
            ("It", "it"),
            ("zh", "zh-CN"),
            ("ZH", "zh-CN"),
            ("zh-cn", "zh-CN"),
            ("zh-CN", "zh-CN"),
            ("ZH-CN", "zh-CN"),
            ("zh_CN", "zh-CN"),
            ("zh_cn", "zh-CN"),
            ("zh-Hans", "zh-CN"),
            ("zh_Hans", "zh-CN"),
            ("zh-Hans-CN", "zh-CN"),
            ("\tzh\n", "zh-CN"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_lang(input), expected, "normalize_lang({input:?})");
        }
    }

    #[test]
    fn normalize_lang_lowercases_and_trims_unknown_codes() {
        let cases = [
            ("PT", "pt"),
            (" ja ", "ja"),
            ("en-US", "en-us"),
            ("pt_BR", "pt_br"),
            // Traditional Chinese is not Simplified Chinese; it must not be
            // silently mapped onto the zh-CN provider set by code alone.
            ("zh-TW", "zh-tw"),
            ("zh-Hant", "zh-hant"),
            ("", ""),
            ("   ", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_lang(input), expected, "normalize_lang({input:?})");
        }
    }

    #[test]
    fn every_supported_code_is_its_own_canonical_form() {
        for code in SUPPORTED_LANGS {
            assert_eq!(normalize_lang(code), *code);
        }
    }

    #[test]
    fn is_chinese_accepts_aliases_only() {
        let cases = [
            ("zh-CN", true),
            ("zh", true),
            ("ZH_cn", true),
            ("zh-Hans", true),
            ("en", false),
            ("zh-TW", false),
            ("", false),
        ];
        for (input, expected) in cases {
            assert_eq!(is_chinese(input), expected, "is_chinese({input:?})");
        }
    }

    #[test]
    fn is_cjk_char_covers_unified_ideographs_and_extension_a() {
        let cases = [
            ('\u{4E00}', true), // 一, first unified ideograph
            ('学', true),
            ('\u{9FFF}', true),  // last code point of the unified block
            ('\u{3400}', true),  // 㐀, first of Extension A
            ('\u{4DB5}', true),  // 䶵
            ('\u{4DBF}', true),  // last of Extension A
            ('\u{33FF}', false), // just below Extension A (CJK compatibility)
            ('\u{4DC0}', false), // ䷀, Yijing hexagram: between the two blocks
            ('\u{A000}', false), // Yi syllable, just above the unified block
            ('\u{3002}', false), // 。ideographic full stop is punctuation
            ('\u{3042}', false), // あ hiragana
            ('\u{AC00}', false), // 가 hangul
            ('a', false),
            ('ü', false),
        ];
        for (c, expected) in cases {
            assert_eq!(
                is_cjk_char(c),
                expected,
                "is_cjk_char({c:?} U+{:04X})",
                c as u32
            );
        }
    }

    #[test]
    fn contains_cjk_and_cjk_chars_scan_the_whole_text() {
        assert!(contains_cjk("学习"));
        assert!(contains_cjk("I like 咖啡 a lot"));
        assert!(contains_cjk("\u{3400}"));
        assert!(!contains_cjk("serendipity"));
        assert!(!contains_cjk("。、！"));
        assert!(!contains_cjk(""));

        assert_eq!(cjk_chars("学习ABC学"), vec!['学', '习', '学']);
        assert_eq!(cjk_chars("a㐀b一"), vec!['㐀', '一']);
        assert!(cjk_chars("no hanzi here").is_empty());
    }

    #[test]
    fn effective_source_lang_sniffs_cjk_before_trusting_config() {
        let cases = [
            ("en", "学习", "zh-CN"),
            ("de", "Kaffee 咖啡", "zh-CN"),
            ("EN", "serendipity", "en"),
            ("zh", "ni hao", "zh-CN"),
            ("DE", "Haus", "de"),
            ("pt-BR", "saudade", "pt-br"),
            ("en", "\u{3400}", "zh-CN"),
        ];
        for (configured, text, expected) in cases {
            assert_eq!(
                effective_source_lang(configured, text),
                expected,
                "effective_source_lang({configured:?}, {text:?})"
            );
        }
    }

    #[test]
    fn is_phrase_looks_for_inner_whitespace_only() {
        let cases = [
            ("serendipity", false),
            ("  serendipity  ", false),
            ("\tword\n", false),
            ("ice cream", true),
            ("  ice cream ", true),
            ("ice\tcream", true),
            ("line\nbreak", true),
            ("学习\u{3000}中文", true), // ideographic space
            ("学习中文", false),
            ("rock'n'roll", false),
            ("", false),
            ("   ", false),
        ];
        for (text, expected) in cases {
            assert_eq!(is_phrase(text), expected, "is_phrase({text:?})");
        }
    }
}
