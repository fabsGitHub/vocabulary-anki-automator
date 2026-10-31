//! Pinyin as text: cleaning, comparison keys and syllable spacing.
//!
//! The two upstreams write the same reading differently. drkameleon's forms
//! come from CC-CEDICT and separate syllables with spaces (`ài hào`); the
//! official list writes words solid with apostrophes and hyphens (`àihào`,
//! `nǚ'ér`, `dì-èr`), capitalizes proper nouns (`Běijīng`) and lists
//! alternatives with a slash (`shéi/shuí`). Form selection compares
//! [`reading_key`]s, which drop all of that, so only the letters and tones
//! decide whether two readings are the same.
//!
//! drkameleon also leaks CC-CEDICT's numbered-pinyin conventions into some
//! tone-marked strings (`lu:è` for `lüè`, a trailing neutral-tone `5` in
//! `yǒukòngr5`, bopomofo's `˙` in `guī ˙nu:`), and writes a handful of
//! non-dictionary entries without spaces (`diàndòngchē`). [`clean_pinyin`] and
//! [`respace_syllables`] repair those, because the bundled pinyin ends up on
//! cards as `ScriptMetadata::Chinese.pinyin`: tone-marked syllables separated
//! by single spaces.
//!
//! Unicode normalization: there is no normalization crate in the approved
//! dependency set, so [`compose_pinyin`] implements canonical composition for
//! exactly the letters pinyin uses. Both pinned upstream files are already in
//! NFC; the composition guards against a decomposed reading slipping in on a
//! future regeneration.

use vocab_core::providers::chinese::generate_pinyin;

const MACRON: char = '\u{0304}';
const ACUTE: char = '\u{0301}';
const CARON: char = '\u{030C}';
const GRAVE: char = '\u{0300}';
const DIAERESIS: char = '\u{0308}';
const CIRCUMFLEX: char = '\u{0302}';

/// Tone-marked letters by base letter, in tone order (1st to 4th); `None`
/// where Unicode has no precomposed character.
const TONE_TABLE: &[(char, [Option<char>; 4])] = &[
    ('a', [Some('ā'), Some('á'), Some('ǎ'), Some('à')]),
    ('e', [Some('ē'), Some('é'), Some('ě'), Some('è')]),
    ('i', [Some('ī'), Some('í'), Some('ǐ'), Some('ì')]),
    ('o', [Some('ō'), Some('ó'), Some('ǒ'), Some('ò')]),
    ('u', [Some('ū'), Some('ú'), Some('ǔ'), Some('ù')]),
    ('ü', [Some('ǖ'), Some('ǘ'), Some('ǚ'), Some('ǜ')]),
    ('A', [Some('Ā'), Some('Á'), Some('Ǎ'), Some('À')]),
    ('E', [Some('Ē'), Some('É'), Some('Ě'), Some('È')]),
    ('I', [Some('Ī'), Some('Í'), Some('Ǐ'), Some('Ì')]),
    ('O', [Some('Ō'), Some('Ó'), Some('Ǒ'), Some('Ò')]),
    ('U', [Some('Ū'), Some('Ú'), Some('Ǔ'), Some('Ù')]),
    ('Ü', [Some('Ǖ'), Some('Ǘ'), Some('Ǚ'), Some('Ǜ')]),
    ('ê', [None, Some('ế'), None, Some('ề')]),
    ('Ê', [None, Some('Ế'), None, Some('Ề')]),
    ('m', [None, Some('ḿ'), None, None]),
    ('M', [None, Some('Ḿ'), None, None]),
    ('n', [None, Some('ń'), Some('ň'), Some('ǹ')]),
    ('N', [None, Some('Ń'), Some('Ň'), Some('Ǹ')]),
];

/// Characters that separate syllables or words without being part of the
/// reading.
const SEPARATORS: &[char] = &['\'', '’', 'ʼ', '-', '‐', '‑', '·'];

/// Canonical composition (NFC) of the pinyin letters: a base letter followed
/// by a combining tone mark (or `u` + diaeresis, `e` + circumflex) becomes the
/// precomposed letter. Everything else is left as it is.
pub fn compose_pinyin(text: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(text.len());
    for c in text.chars() {
        if let Some(last) = out.last_mut() {
            if let Some(composed) = compose(*last, c) {
                *last = composed;
                continue;
            }
        }
        out.push(c);
    }
    out.into_iter().collect()
}

fn compose(base: char, mark: char) -> Option<char> {
    let tone = match mark {
        MACRON => 0,
        ACUTE => 1,
        CARON => 2,
        GRAVE => 3,
        DIAERESIS => {
            return match base {
                'u' => Some('ü'),
                'U' => Some('Ü'),
                _ => None,
            }
        }
        CIRCUMFLEX => {
            return match base {
                'e' => Some('ê'),
                'E' => Some('Ê'),
                _ => None,
            }
        }
        _ => return None,
    };
    TONE_TABLE
        .iter()
        .find(|(letter, _)| *letter == base)
        .and_then(|(_, tones)| tones[tone])
}

/// The letter without its tone mark: `ǎ` -> `a`, `ǜ` -> `ü`, `ň` -> `n`.
pub fn strip_tone(c: char) -> char {
    TONE_TABLE
        .iter()
        .find(|(_, tones)| tones.contains(&Some(c)))
        .map_or(c, |(base, _)| *base)
}

/// Repairs the CC-CEDICT leftovers in a tone-marked reading (see the module
/// docs): `u:` -> `ü`, the bopomofo neutral-tone dot dropped, a neutral-tone
/// `5` at the end of a syllable dropped, whitespace collapsed to single
/// spaces, letters composed.
pub fn clean_pinyin(raw: &str) -> String {
    let fixed = raw
        .replace("u:", "ü")
        .replace("U:", "Ü")
        .replace('\u{02D9}', "");
    let tokens: Vec<&str> = fixed
        .split_whitespace()
        .map(|token| {
            let mut chars = token.chars().rev();
            match (chars.next(), chars.next()) {
                (Some('5'), Some(before)) if before.is_alphabetic() => &token[..token.len() - 1],
                _ => token,
            }
        })
        .collect();
    compose_pinyin(&tokens.join(" "))
}

/// The key two readings are compared by: [`clean_pinyin`], lowercased, with
/// whitespace and [`SEPARATORS`] removed. `Bù kèqi`, `bù kèqi` and `bùkèqi`
/// share a key.
pub fn reading_key(pinyin: &str) -> String {
    reading_key_with_case(pinyin).to_lowercase()
}

/// [`reading_key`] without lowercasing, to prefer `zhōng` over the proper-noun
/// form `Zhōng` when both match an official `zhōng`.
pub fn reading_key_with_case(pinyin: &str) -> String {
    clean_pinyin(pinyin)
        .chars()
        .filter(|c| !c.is_whitespace() && !SEPARATORS.contains(c))
        .collect()
}

/// How far two [`reading_key`]s are apart when the only differences are
/// neutral tones: the number of letters that carry a tone mark on one side
/// and none on the other (`hǎochu` vs `hǎochù` is 1). `None` when they differ
/// in anything else, including two different tone marks (`chǔ` vs `chù`).
///
/// CC-CEDICT writes many second syllables in the neutral tone (`hǎo chu`,
/// `guān xi`) where the official list writes the citation tone (`hǎochù`,
/// `guānxì`); this is what lets form selection see that they are the same
/// reading.
pub fn neutral_tone_distance(a: &str, b: &str) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len() != b.len() {
        return None;
    }
    let mut distance = 0;
    for (&x, &y) in a.iter().zip(&b) {
        if x == y {
            continue;
        }
        let (base_x, base_y) = (strip_tone(x), strip_tone(y));
        if base_x != base_y || (x != base_x && y != base_y) {
            return None;
        }
        distance += 1;
    }
    Some(distance)
}

/// Splits a reading written without spaces (`diàndòngchē`) into one
/// space-separated syllable per character of `hanzi` (`diàn dòng chē`).
///
/// Each character's default reading from the `pinyin` crate is matched,
/// ignoring tones and case, against the front of what is left; a 儿 that
/// only adds an `r` (erhua: `xiàohuar` for 笑话儿) is attached to the
/// syllable before it. `None` when the reading already has a space per
/// syllable or when any character does not line up, in which case the
/// caller keeps the original: a guessed split would be worse than none.
pub fn respace_syllables(pinyin: &str, hanzi: &str) -> Option<String> {
    let hanzi: Vec<char> = hanzi.chars().collect();
    if pinyin.split_whitespace().count() >= hanzi.len() {
        return None;
    }
    let letters: Vec<char> = pinyin.chars().filter(|c| !c.is_whitespace()).collect();
    if letters.iter().any(|c| !c.is_alphabetic()) {
        return None;
    }
    let plain: Vec<char> = letters.iter().map(|c| plain_letter(*c)).collect();
    let mut syllables: Vec<String> = Vec::new();
    let mut pos = 0;
    for &character in &hanzi {
        let rest = &plain[pos..];
        let erhua = character == '儿'
            && rest.first() == Some(&'r')
            && !rest.get(1).is_some_and(|next| is_vowel(*next));
        if erhua {
            if let Some(previous) = syllables.last_mut() {
                previous.push(letters[pos]);
                pos += 1;
                continue;
            }
        }
        let reading: Vec<char> = generate_pinyin(&character.to_string())
            .chars()
            .map(plain_letter)
            .collect();
        if reading.is_empty() || !rest.starts_with(&reading) {
            return None;
        }
        syllables.push(letters[pos..pos + reading.len()].iter().collect());
        pos += reading.len();
    }
    let respaced = syllables.join(" ");
    (pos == letters.len() && syllables.len() > pinyin.split_whitespace().count())
        .then_some(respaced)
}

/// Lowercase letter without tone mark, for matching readings.
fn plain_letter(c: char) -> char {
    strip_tone(c).to_lowercase().next().unwrap_or(c)
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'ü' | 'ê')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composes_decomposed_tone_marks() {
        let cases = [
            ("a\u{304}", "ā"),
            ("xue\u{301}", "xué"),
            ("ni\u{30C}", "nǐ"),
            ("ka\u{300}n", "kàn"),
            ("lu\u{308}e\u{300}", "lüè"),
            ("nu\u{308}\u{30C}", "nǚ"),
            ("Be\u{30C}iji\u{304}ng", "Běijīng"),
            ("n\u{30C}", "ň"),
            ("e\u{302}\u{301}", "ế"),
            // No precomposed form: left decomposed, as NFC would.
            ("m\u{304}", "m\u{304}"),
            ("x\u{301}", "x\u{301}"),
            ("ài hào", "ài hào"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(compose_pinyin(input), expected, "{input:?}");
        }
    }

    #[test]
    fn strips_tones_to_the_base_letter() {
        let cases = [
            ('ā', 'a'),
            ('ǘ', 'ü'),
            ('Ǚ', 'Ü'),
            ('ň', 'n'),
            ('ü', 'ü'),
            ('x', 'x'),
        ];
        for (input, expected) in cases {
            assert_eq!(strip_tone(input), expected, "{input:?}");
        }
    }

    #[test]
    fn cleans_cedict_leftovers() {
        let cases = [
            ("lu:è", "lüè"),
            ("cè lu:è", "cè lüè"),
            ("Lu: Xùn", "Lü Xùn"),
            ("guī ˙nu:", "guī nü"),
            ("yǒukòngr5", "yǒukòngr"),
            ("bù yīhuǐr5", "bù yīhuǐr"),
            ("  ài   hào ", "ài hào"),
            // A lone digit is not a tone suffix, and other digits are kept
            // so the summary can flag them.
            ("5", "5"),
            ("ka3", "ka3"),
            ("xue\u{301}", "xué"),
        ];
        for (input, expected) in cases {
            assert_eq!(clean_pinyin(input), expected, "{input:?}");
        }
    }

    #[test]
    fn reading_keys_ignore_spacing_separators_and_case() {
        let same = [
            ("ài hào", "àihào"),
            ("nǚ ér", "nǚ'ér"),
            ("dì èr", "dì-èr"),
            ("yī lù píng ān", "yīlù-píng'ān"),
            ("Běi jīng", "Běijīng"),
            ("Zhōng", "zhōng"),
            ("cè lu:è", "cèlüè"),
            ("xue\u{301}", "xué"),
        ];
        for (a, b) in same {
            assert_eq!(reading_key(a), reading_key(b), "{a:?} vs {b:?}");
        }
        let different = [("ba", "bā"), ("de", "dì"), ("hái", "huán"), ("lü", "lu")];
        for (a, b) in different {
            assert_ne!(reading_key(a), reading_key(b), "{a:?} vs {b:?}");
        }
        assert_ne!(
            reading_key_with_case("Zhōng"),
            reading_key_with_case("zhōng")
        );
        assert_eq!(reading_key_with_case("Běi jīng"), "Běijīng");
    }

    #[test]
    fn neutral_tones_are_a_distance_other_differences_are_not() {
        let key = |p: &str| reading_key(p);
        let cases = [
            ("hǎo chu", "hǎochù", Some(1)),
            ("guān xi", "guānxì", Some(1)),
            ("ě xin", "ěxīn", Some(1)),
            ("qi lai", "qǐlái", Some(2)),
            ("hǎo chù", "hǎochù", Some(0)),
            // Two different tones are a different reading.
            ("hǎo chǔ", "hǎochù", None),
            ("è xīn", "ěxin", None),
            ("fēn liàng", "fènliàng", None),
            // Different letters or lengths.
            ("lu", "lü", None),
            ("hǎo", "hǎochù", None),
        ];
        for (a, b, expected) in cases {
            assert_eq!(
                neutral_tone_distance(&key(a), &key(b)),
                expected,
                "{a:?} vs {b:?}"
            );
            assert_eq!(
                neutral_tone_distance(&key(b), &key(a)),
                expected,
                "{b:?} vs {a:?}"
            );
        }
    }

    #[test]
    fn respaces_solid_readings_one_syllable_per_character() {
        let cases = [
            ("diàndòngchē", "电动车", Some("diàn dòng chē")),
            ("nányǐxiǎngxiàng", "难以想象", Some("nán yǐ xiǎng xiàng")),
            ("jiànguò", "见过", Some("jiàn guò")),
            // Neutral tones and other tones than the default reading still
            // line up: only the letters are compared.
            ("yǎnli", "眼里", Some("yǎn li")),
            // Erhua: the r joins the syllable before it.
            ("xiàohuar", "笑话儿", Some("xiào huar")),
            ("bù yīhuǐr", "不一会儿", Some("bù yī huǐr")),
            // 儿 as a syllable of its own.
            ("nǚér", "女儿", Some("nǚ ér")),
            // Proper nouns keep their capital.
            ("Běijīng", "北京", Some("Běi jīng")),
        ];
        for (pinyin, hanzi, expected) in cases {
            assert_eq!(
                respace_syllables(pinyin, hanzi).as_deref(),
                expected,
                "{pinyin:?} for {hanzi}"
            );
        }
    }

    #[test]
    fn respacing_gives_up_rather_than_guessing() {
        let cases = [
            // Already one token per syllable (erhua words keep theirs).
            ("ài hào", "爱好"),
            ("yī huìr", "一会儿"),
            ("xué", "学"),
            // A reading that does not line up with the characters.
            ("xuéxí", "银行"),
            ("diàndòng", "电动车"),
            ("diàndòngchēzi", "电动车"),
            // Separators and digits are not guessed around.
            ("nǚ'ér", "女儿"),
            ("ka3la1", "卡拉"),
        ];
        for (pinyin, hanzi) in cases {
            assert_eq!(
                respace_syllables(pinyin, hanzi),
                None,
                "{pinyin:?} for {hanzi}"
            );
        }
    }
}
