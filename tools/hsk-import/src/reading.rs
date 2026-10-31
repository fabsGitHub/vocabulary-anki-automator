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
//! `yǒukòngr5`, bopomofo's `˙` in `guī ˙nu:`, a tone mark on the wrong vowel
//! in `Aò` for `Ào`), writes a handful of non-dictionary entries without
//! spaces (`diàndòngchē`), and spells erhua both ways: `yī diǎn r` with
//! CC-CEDICT's lone `r`, `yǒukòngr5` with the `r` attached. [`clean_pinyin`],
//! [`respace_syllables`] and [`join_erhua`] repair those, because the bundled
//! pinyin ends up on cards as `ScriptMetadata::Chinese.pinyin`: tone-marked
//! syllables separated by single spaces, an erhua `r` being part of its
//! syllable as in the official list (`yīdiǎnr`).
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

/// The tone (0 = first .. 3 = fourth) of a tone-marked letter.
fn tone_of(c: char) -> Option<usize> {
    TONE_TABLE
        .iter()
        .find_map(|(_, tones)| tones.iter().position(|tone| *tone == Some(c)))
}

/// `base` with tone `tone` (0 = first .. 3 = fourth), if Unicode has it.
fn with_tone(base: char, tone: usize) -> Option<char> {
    TONE_TABLE
        .iter()
        .find(|(letter, _)| *letter == base)
        .and_then(|(_, tones)| tones[tone])
}

/// Repairs the CC-CEDICT leftovers in a tone-marked reading (see the module
/// docs): `u:` -> `ü`, the bopomofo neutral-tone dot dropped, a neutral-tone
/// `5` at the end of a syllable dropped, a misplaced tone mark moved
/// ([`place_tone_mark`]), whitespace collapsed to single spaces, letters
/// composed.
pub fn clean_pinyin(raw: &str) -> String {
    let fixed = compose_pinyin(
        &raw.replace("u:", "ü")
            .replace("U:", "Ü")
            .replace('\u{02D9}', ""),
    );
    let tokens: Vec<String> = fixed
        .split_whitespace()
        .map(|token| {
            let mut chars = token.chars().rev();
            let token = match (chars.next(), chars.next()) {
                (Some('5'), Some(before)) if before.is_alphabetic() => &token[..token.len() - 1],
                _ => token,
            };
            place_tone_mark(token)
        })
        .collect();
    tokens.join(" ")
}

/// Moves the tone mark of a one-syllable token to the vowel the pinyin
/// rules put it on: `a` or `e` when there is one, the `o` of `ou`, else the
/// last vowel (`Aò` -> `Ào`, `líu` -> `liú`). A token with more or fewer
/// than one mark, or whose vowels are not one contiguous run (`yǎnli` is two
/// syllables), is returned unchanged: where its syllables start is unknown.
fn place_tone_mark(token: &str) -> String {
    let chars: Vec<char> = token.chars().collect();
    let plain: Vec<char> = chars.iter().map(|&c| plain_letter(c)).collect();
    let marked: Vec<usize> = (0..chars.len())
        .filter(|&i| strip_tone(chars[i]) != chars[i])
        .collect();
    let vowels: Vec<usize> = (0..chars.len()).filter(|&i| is_vowel(plain[i])).collect();
    let (&[marked], Some(&last)) = (marked.as_slice(), vowels.last()) else {
        return token.to_string();
    };
    if !is_one_syllable(&plain) {
        return token.to_string();
    }
    let target = vowels
        .iter()
        .copied()
        .find(|&i| plain[i] == 'a')
        .or_else(|| {
            vowels
                .iter()
                .copied()
                .find(|&i| matches!(plain[i], 'e' | 'ê'))
        })
        .or_else(|| {
            vowels
                .iter()
                .copied()
                .find(|&i| plain[i] == 'o' && plain.get(i + 1) == Some(&'u'))
        })
        .unwrap_or(last);
    let moved = tone_of(chars[marked]).and_then(|tone| with_tone(chars[target], tone));
    match moved {
        Some(moved) if target != marked => {
            let mut fixed = chars;
            fixed[marked] = strip_tone(fixed[marked]);
            fixed[target] = moved;
            fixed.into_iter().collect()
        }
        _ => token.to_string(),
    }
}

/// Joins a lone erhua `r` (CC-CEDICT's 一点儿 `yī diǎn r`) to the syllable
/// before it (`yī diǎnr`), as the official list and [`respace_syllables`]
/// write it. `None` when there is nothing to join or the reading does not
/// have one token per character of `hanzi`.
pub fn join_erhua(pinyin: &str, hanzi: &str) -> Option<String> {
    let tokens: Vec<&str> = pinyin.split_whitespace().collect();
    let characters: Vec<char> = hanzi.chars().collect();
    if tokens.len() != characters.len() {
        return None;
    }
    let mut syllables: Vec<String> = Vec::with_capacity(tokens.len());
    let mut joined = false;
    for (token, &character) in tokens.iter().zip(&characters) {
        if character == '儿' && token.eq_ignore_ascii_case("r") {
            if let Some(previous) = syllables.last_mut() {
                previous.push_str(token);
                joined = true;
                continue;
            }
        }
        syllables.push(token.to_string());
    }
    joined.then(|| syllables.join(" "))
}

/// How many syllables `pinyin` has fewer than `hanzi` has characters,
/// beyond the 儿 that erhua merges into the syllable before: 0 for a reading
/// with one syllable per character, more for one written (partly) solid.
pub fn missing_syllables(pinyin: &str, hanzi: &str) -> usize {
    let characters: Vec<char> = hanzi.chars().collect();
    let erhua = characters.iter().skip(1).filter(|&&c| c == '儿').count();
    (characters.len() - erhua).saturating_sub(pinyin.split_whitespace().count())
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
/// syllable before it. One character whose default reading is not the one
/// the word uses (着 reads `zhe` by default, `zhuó` in 着眼于) is allowed
/// when every character after it lines up from the end: it gets the letters
/// in between. `None` when the reading already has a space per syllable or
/// when the characters do not line up, in which case the caller keeps the
/// original: a guessed split would be worse than none.
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
    for (index, &character) in hanzi.iter().enumerate() {
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
        let reading = default_reading(character);
        if reading.is_empty() || !rest.starts_with(&reading) {
            let tail = lengths_from_end(&hanzi[index + 1..], rest)?;
            let middle = rest.len() - tail.iter().sum::<usize>();
            if !is_one_syllable(&rest[..middle]) {
                return None;
            }
            for length in std::iter::once(middle).chain(tail) {
                syllables.push(letters[pos..pos + length].iter().collect());
                pos += length;
            }
            break;
        }
        syllables.push(letters[pos..pos + reading.len()].iter().collect());
        pos += reading.len();
    }
    let respaced = syllables.join(" ");
    (pos == letters.len() && syllables.len() > pinyin.split_whitespace().count())
        .then_some(respaced)
}

/// A character's default reading from the `pinyin` crate as plain letters.
fn default_reading(character: char) -> Vec<char> {
    generate_pinyin(&character.to_string())
        .chars()
        .map(plain_letter)
        .collect()
}

/// The letter counts of `characters`, matched by their default readings
/// against the end of `plain`, in order; `None` when one does not line up
/// (or is a 儿, whose erhua `r` this does not guess at).
fn lengths_from_end(characters: &[char], plain: &[char]) -> Option<Vec<usize>> {
    let mut end = plain.len();
    let mut lengths = Vec::with_capacity(characters.len());
    for &character in characters.iter().rev() {
        let reading = default_reading(character);
        if character == '儿' || reading.is_empty() || !plain[..end].ends_with(&reading) {
            return None;
        }
        end -= reading.len();
        lengths.push(reading.len());
    }
    lengths.reverse();
    Some(lengths)
}

/// Lowercase letter without tone mark, for matching readings.
fn plain_letter(c: char) -> char {
    strip_tone(c).to_lowercase().next().unwrap_or(c)
}

/// Whether plain letters can be one syllable: their vowels are one
/// contiguous, non-empty run (`zhuo`, `er`; not `chezi`).
fn is_one_syllable(plain: &[char]) -> bool {
    let vowels: Vec<usize> = (0..plain.len()).filter(|&i| is_vowel(plain[i])).collect();
    match (vowels.first(), vowels.last()) {
        (Some(&first), Some(&last)) => last - first + 1 == vowels.len(),
        _ => false,
    }
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
            // A tone mark on the wrong vowel of a syllable moves.
            ("Aò yùn huì", "Ào yùn huì"),
            ("líu", "liú"),
            ("dúo", "duó"),
            ("a\u{300}o", "ào"),
            // Correct marks, several syllables in one token, and syllables
            // without a vowel stay as they are.
            ("Ōu zhōu", "Ōu zhōu"),
            ("guì lüè xióng", "guì lüè xióng"),
            ("yǎnli", "yǎnli"),
            ("xiàohuar5", "xiàohuar"),
            ("ň", "ň"),
        ];
        for (input, expected) in cases {
            assert_eq!(clean_pinyin(input), expected, "{input:?}");
        }
    }

    #[test]
    fn a_lone_erhua_r_joins_the_syllable_before_it() {
        let joined = [
            ("yī diǎn r", "一点儿", "yī diǎnr"),
            ("nǎ r", "哪儿", "nǎr"),
            ("yī huì r", "一会儿", "yī huìr"),
        ];
        for (pinyin, hanzi, expected) in joined {
            assert_eq!(
                join_erhua(pinyin, hanzi).as_deref(),
                Some(expected),
                "{pinyin:?}"
            );
        }
        for (pinyin, hanzi) in [
            // 儿 as a syllable of its own, or already joined.
            ("nǚ ér", "女儿"),
            ("ér zi", "儿子"),
            ("yī diǎnr", "一点儿"),
            // Nothing to join it to, or no token per character.
            ("r", "儿"),
            ("yī diǎn", "一点儿"),
        ] {
            assert_eq!(join_erhua(pinyin, hanzi), None, "{pinyin:?} for {hanzi}");
        }
    }

    #[test]
    fn missing_syllables_counts_solid_parts_but_not_erhua() {
        let cases = [
            ("xué xí", "学习", 0),
            ("yī diǎnr", "一点儿", 0),
            ("nǎr", "哪儿", 0),
            ("nǚ ér", "女儿", 0),
            ("zhuóyǎn yú", "着眼于", 1),
            ("diàndòngchē", "电动车", 2),
        ];
        for (pinyin, hanzi, expected) in cases {
            assert_eq!(missing_syllables(pinyin, hanzi), expected, "{pinyin:?}");
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
            // One heteronym whose default reading is another one (着 zhe,
            // 行 xíng, 长 zhǎng) gets the letters the others leave.
            ("zhuóyǎn yú", "着眼于", Some("zhuó yǎn yú")),
            ("yínháng", "银行", Some("yín háng")),
            ("chángchéng", "长城", Some("cháng chéng")),
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
            // The odd character out may take one syllable, never two.
            ("diàndòngchēzi", "电动机"),
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
