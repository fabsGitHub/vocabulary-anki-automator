//! Cutting sentences into word tiles (Milestone M2), and finding a word in a
//! sentence.
//!
//! Chinese is written without spaces, so a sentence becomes tiles by greedy
//! longest match against a dictionary. That dictionary is the learned pool's
//! fronts and nothing else (§ 7.2). With the HSK curriculum as dictionary,
//! `学生很喜欢学习中文` would yield the tile `学生` for a learner who has only
//! learned `学习`: a word they have never seen, made of characters they know.
//! Where no learned word matches, the sentence falls back to single
//! characters, which by then are known to be learned characters. Sentences
//! without hanzi are cut at whitespace instead.
//!
//! Punctuation, symbols and whitespace never become tiles: "word characters"
//! are letters, digits and combining marks (so a decomposed `é` keeps its
//! accent), everything else only separates tiles.

use std::{collections::HashSet, ops::Range};

use super::LearnedVocabularyPool;
use crate::language::{contains_cjk, is_cjk_char};

/// The words a Chinese sentence is cut into: the learned fronts that can
/// stand as one tile, i.e. that contain a hanzi and consist of word
/// characters only (which rules out patterns such as `一…就…`).
#[derive(Debug)]
pub(crate) struct Dictionary {
    words: HashSet<String>,
    /// Length of the longest word, in characters.
    longest: usize,
}

impl Dictionary {
    pub(crate) fn from_pool(pool: &LearnedVocabularyPool) -> Self {
        Self::from_fronts(pool.entries().iter().map(|entry| entry.front.as_str()))
    }

    fn from_fronts<'a>(fronts: impl Iterator<Item = &'a str>) -> Self {
        let words: HashSet<String> = fronts
            .map(str::trim)
            .filter(|front| contains_cjk(front) && front.chars().all(is_word_char))
            .map(str::to_owned)
            .collect();
        let longest = words
            .iter()
            .map(|word| word.chars().count())
            .max()
            .unwrap_or(0);
        Self { words, longest }
    }

    /// Whether `word` can be a tile of a Chinese sentence.
    pub(crate) fn contains(&self, word: &str) -> bool {
        self.words.contains(word)
    }

    /// The length, in characters, of the longest word `chars` starts with.
    fn longest_prefix(&self, chars: &[char]) -> Option<usize> {
        (1..=self.longest.min(chars.len())).rev().find(|&len| {
            self.words
                .contains(&chars[..len].iter().collect::<String>())
        })
    }
}

/// The tiles of `sentence`, in sentence order: greedy longest match against
/// `dictionary` when the sentence contains a hanzi, whitespace-separated
/// words with their surrounding punctuation stripped otherwise.
pub(crate) fn tiles(sentence: &str, dictionary: &Dictionary) -> Vec<String> {
    if contains_cjk(sentence) {
        cjk_tiles(sentence, dictionary)
    } else {
        sentence.split_whitespace().filter_map(word_tile).collect()
    }
}

/// Greedy longest match from left to right. Where no dictionary word starts,
/// a hanzi becomes a tile of its own and a run of other word characters
/// (`iPhone` in `我用iPhone`) one tile together.
fn cjk_tiles(sentence: &str, dictionary: &Dictionary) -> Vec<String> {
    let chars: Vec<char> = sentence.chars().collect();
    let mut tiles = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let rest = &chars[start..];
        if !is_word_char(rest[0]) {
            start += 1;
            continue;
        }
        let len = dictionary.longest_prefix(rest).unwrap_or_else(|| {
            if is_cjk_char(rest[0]) {
                1
            } else {
                rest.iter()
                    .take_while(|c| is_word_char(**c) && !is_cjk_char(**c))
                    .count()
            }
        });
        tiles.push(rest[..len].iter().collect());
        start += len;
    }
    tiles
}

/// One whitespace-separated word without its surrounding punctuation
/// (`„Haus“,` -> `Haus`; `don't` keeps its apostrophe); `None` when nothing
/// is left.
pub(crate) fn word_tile(token: &str) -> Option<String> {
    let word = token.trim_matches(|c: char| !is_word_char(c));
    (!word.is_empty()).then(|| word.to_owned())
}

/// Letters, digits and combining marks.
pub(crate) fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c)
}

/// The combining-mark blocks (diacritics, their extensions and supplement,
/// marks for symbols, half marks).
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// The byte range of the first occurrence of `word` (trimmed) in `text`:
/// exact when `word` contains a hanzi, ignoring case otherwise. A blank word
/// occurs nowhere.
///
/// Outside hanzi, an occurrence that stands as a word of its own wins over
/// an earlier one inside a longer word, so `cat` in "Concatenate the cat" is
/// the last three letters; only when the word never stands alone (`run` in
/// "running") is the first occurrence used. Hanzi have no word boundaries to
/// go by, so there the first occurrence is always the one.
///
/// Case is compared one character at a time, so the range always covers as
/// many characters of `text` as `word` has, and slicing `text` with it never
/// splits a character (whole-string lowercasing can change lengths, e.g.
/// `İ` -> `i̇`).
pub(crate) fn find_word(text: &str, word: &str) -> Option<Range<usize>> {
    let word = word.trim();
    if word.is_empty() {
        return None;
    }
    if contains_cjk(word) {
        return text.find(word).map(|start| start..start + word.len());
    }
    let mut occurrences = text.char_indices().filter_map(|(start, _)| {
        let mut end = start;
        let mut rest = text[start..].chars();
        for expected in word.chars() {
            let found = rest.next().filter(|found| same_letter(*found, expected))?;
            end += found.len_utf8();
        }
        Some(start..end)
    });
    let first = occurrences.next()?;
    if stands_alone(text, &first) {
        return Some(first);
    }
    Some(
        occurrences
            .find(|range| stands_alone(text, range))
            .unwrap_or(first),
    )
}

/// Whether `range` of `text` is neither preceded nor followed by a word
/// character.
fn stands_alone(text: &str, range: &Range<usize>) -> bool {
    let before = text[..range.start].chars().next_back();
    let after = text[range.end..].chars().next();
    !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char)
}

fn same_letter(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dictionary(words: &[&str]) -> Dictionary {
        Dictionary::from_fronts(words.iter().copied())
    }

    #[test]
    fn chinese_is_cut_by_greedy_longest_match_with_single_character_fallback() {
        let dict = dictionary(&["我", "我们", "学习", "中文", "喜欢"]);
        let cases: [(&str, &[&str]); 6] = [
            ("我们喜欢学习中文。", &["我们", "喜欢", "学习", "中文"]),
            // 学生 is not a learned word: its characters come out one by one.
            (
                "学生很喜欢学习中文。",
                &["学", "生", "很", "喜欢", "学习", "中文"],
            ),
            ("我，我们！", &["我", "我们"]),
            ("  我们 \u{3000}学习  ", &["我们", "学习"]),
            ("我用iPhone学习", &["我", "用", "iPhone", "学习"]),
            ("「中文」……", &["中文"]),
        ];
        for (sentence, expected) in cases {
            assert_eq!(tiles(sentence, &dict), expected, "{sentence:?}");
        }
    }

    #[test]
    fn longest_match_wins_over_a_shorter_prefix() {
        let dict = dictionary(&["中", "中文", "中文课", "文"]);
        assert_eq!(tiles("中文课中文中", &dict), ["中文课", "中文", "中"]);
    }

    #[test]
    fn dictionary_keeps_only_fronts_that_can_be_one_tile() {
        let dict = dictionary(&[" 学习 ", "一…就…", "serendipity", "AA制", "你 好", ""]);
        assert!(dict.contains("学习"));
        assert!(dict.contains("AA制"));
        for rejected in ["一…就…", "serendipity", "你 好", ""] {
            assert!(!dict.contains(rejected), "{rejected:?}");
        }
        assert_eq!(dict.longest, 3);
        assert_eq!(tiles("我们AA制", &dict), ["我", "们", "AA制"]);
    }

    #[test]
    fn other_scripts_are_cut_at_whitespace_without_surrounding_punctuation() {
        let dict = dictionary(&["学习"]);
        let cases: [(&str, &[&str]); 5] = [
            (
                "It was pure serendipity.",
                &["It", "was", "pure", "serendipity"],
            ),
            (
                "We use the expression 'serendipity' in everyday context.",
                &[
                    "We",
                    "use",
                    "the",
                    "expression",
                    "serendipity",
                    "in",
                    "everyday",
                    "context",
                ],
            ),
            ("„Das Haus“ – sagt er.", &["Das", "Haus", "sagt", "er"]),
            (
                "¿Dónde está? Qu'est-ce que c'est ?",
                &["Dónde", "está", "Qu'est-ce", "que", "c'est"],
            ),
            ("cafe\u{0301} au lait", &["cafe\u{0301}", "au", "lait"]),
        ];
        for (sentence, expected) in cases {
            assert_eq!(tiles(sentence, &dict), expected, "{sentence:?}");
        }
    }

    #[test]
    fn find_word_is_exact_for_hanzi_and_case_insensitive_otherwise() {
        let cases: [(&str, &str, Option<&str>); 9] = [
            ("我喜欢学习中文。", "学习", Some("学习")),
            ("我喜欢學習中文。", "学习", None),
            ("Pure serendipity.", "Serendipity", Some("serendipity")),
            ("ÜBER alles", "über", Some("ÜBER")),
            ("The cat sat.", " cat ", Some("cat")),
            ("concatenate the cat", "cat", Some("cat")),
            ("no match", "word", None),
            ("anything", "  ", None),
            // ẞ and ß differ in UTF-8 length; the range still ends on a boundary.
            ("STRAẞE und Straße", "straße", Some("STRAẞE")),
        ];
        for (text, word, expected) in cases {
            let found = find_word(text, word).map(|range| &text[range]);
            assert_eq!(found, expected, "find_word({text:?}, {word:?})");
        }
    }

    #[test]
    fn find_word_prefers_an_occurrence_that_stands_alone() {
        let cases: [(&str, &str, Option<std::ops::Range<usize>>); 7] = [
            // The word itself, not the letters inside "concatenate".
            ("concatenate the cat", "cat", Some(16..19)),
            ("Concatenate the CAT!", "cat", Some(16..19)),
            ("cat, concatenate", "cat", Some(0..3)),
            // Never alone: the first occurrence inside a longer word.
            ("He is running.", "run", Some(6..9)),
            ("concatenate", "cat", Some(3..6)),
            // Punctuation and the ends of the text are boundaries.
            ("„Haus“ und Haustür", "haus", Some(3..7)),
            // Hanzi have no boundaries: always the first occurrence.
            ("学习学", "学", Some(0..3)),
        ];
        for (text, word, expected) in cases {
            assert_eq!(
                find_word(text, word),
                expected,
                "find_word({text:?}, {word:?})"
            );
        }
    }
}
