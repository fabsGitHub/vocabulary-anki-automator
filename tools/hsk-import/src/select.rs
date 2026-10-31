//! Which of a word's forms becomes its curriculum entry.
//!
//! The plan said to take each entry's first form. drkameleon lists forms in
//! CC-CEDICT's dictionary order, though, and for many of the most common
//! words the first form is not the HSK word: 吧 starts with the loanword `bā`
//! ("bar"), 还 with the surname `Huán`, 中 with the proper noun `Zhōng`. Taking
//! the first form would put wrong pinyin and meanings on exactly the words a
//! learner meets first. The rules, in order:
//!
//! 1. the form whose reading equals the official reading of the word at its
//!    lowest official level (lowest ID on a tie), compared by
//!    [`reading_key`]; among several matches, one with the official
//!    capitalization, then one not dominated by penalty meanings;
//!
//!    1b. else the form whose reading differs from that official reading only
//!    in neutral tones ([`neutral_tone_distance`]), fewest differences first,
//!    then by the same preferences. CC-CEDICT writes `hǎo chu` and `ě xin`
//!    where the official list has `hǎochù` and `ěxin`; without this step those
//!    words fell through to rules 2 and 3, which gave 好处 `hǎo chǔ` ("easy to
//!    get along with"), 恶心 `è xīn` ("vice") and 分量 `fēn liàng` ("vector
//!    component") instead of the HSK words;
//! 2. else the form whose reading equals the `pinyin` crate's default
//!    reading of the word (one reading per character, joined by spaces);
//! 3. else the first form not dominated by penalty meanings;
//! 4. else the first form.
//!
//! A *penalty meaning* is a cross-reference or a marginal sense rather than
//! the word itself: it contains `surname`, `variant of`, `old variant`,
//! `(onom.)`, `abbr. for`, `(loanword)`, `erhua variant` or `Japanese
//! variant`, or starts with `used in` or `see `. Those two only count at the
//! start because mid-sentence they are usage notes on real senses (比:
//! "(used in comparison) than"). A form is *dominated* when it has no
//! meanings or more than half of them are penalty meanings.

use vocab_core::providers::chinese::generate_pinyin;

use crate::{
    official::OfficialReading,
    reading::{neutral_tone_distance, reading_key, reading_key_with_case},
    vocabulary::Form,
};

/// Markers that make a meaning a penalty meaning wherever they appear
/// (compared case-insensitively).
pub const PENALTY_MARKERS: &[&str] = &[
    "surname",
    "variant of",
    "old variant",
    "(onom.)",
    "abbr. for",
    "(loanword)",
    "erhua variant",
    "japanese variant",
];

/// Markers that only count at the start of a meaning.
pub const PENALTY_PREFIXES: &[&str] = &["used in", "see "];

/// The rule that decided a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rule {
    /// The entry has one usable form; there was nothing to choose.
    SingleForm,
    /// Rule 1: the official HSK reading.
    OfficialReading,
    /// Rule 1b: the official HSK reading up to neutral tones.
    OfficialReadingNeutralTone,
    /// Rule 2: the `pinyin` crate's default reading.
    DefaultReading,
    /// Rule 3: the first form not dominated by penalty meanings.
    NotDominated,
    /// Rule 4: the first form.
    FirstForm,
}

impl Rule {
    /// The rules for entries with a choice, in order.
    pub const CHOICES: [Rule; 5] = [
        Rule::OfficialReading,
        Rule::OfficialReadingNeutralTone,
        Rule::DefaultReading,
        Rule::NotDominated,
        Rule::FirstForm,
    ];

    pub fn describe(&self) -> &'static str {
        match self {
            Rule::SingleForm => "single form",
            Rule::OfficialReading => "rule 1 (official reading)",
            Rule::OfficialReadingNeutralTone => "rule 1b (official reading up to neutral tones)",
            Rule::DefaultReading => "rule 2 (pinyin crate default reading)",
            Rule::NotDominated => "rule 3 (first form not dominated by penalty meanings)",
            Rule::FirstForm => "rule 4 (first form)",
        }
    }
}

/// The chosen form (an index into the entry's `forms`) and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub index: usize,
    pub rule: Rule,
}

pub fn is_penalty_meaning(meaning: &str) -> bool {
    let meaning = meaning.trim().to_lowercase();
    PENALTY_MARKERS
        .iter()
        .any(|marker| meaning.contains(marker))
        || PENALTY_PREFIXES
            .iter()
            .any(|prefix| meaning.starts_with(prefix))
}

/// No meanings at all, or more than half of them penalty meanings.
pub fn is_dominated(form: &Form) -> bool {
    let meanings: Vec<&String> = form
        .meanings
        .iter()
        .filter(|meaning| !meaning.trim().is_empty())
        .collect();
    let penalties = meanings
        .iter()
        .filter(|meaning| is_penalty_meaning(meaning))
        .count();
    meanings.is_empty() || penalties * 2 > meanings.len()
}

/// Picks `hanzi`'s form by the rules in the module docs. Forms without a
/// reading are never picked; `None` when no form has one.
pub fn select_form(
    hanzi: &str,
    forms: &[Form],
    official: Option<&OfficialReading>,
) -> Option<Selection> {
    let usable: Vec<usize> = (0..forms.len())
        .filter(|&i| !forms[i].transcriptions.pinyin.trim().is_empty())
        .collect();
    let first = *usable.first()?;
    if usable.len() == 1 {
        return Some(Selection {
            index: first,
            rule: Rule::SingleForm,
        });
    }
    let chosen = |index, rule| Some(Selection { index, rule });

    if let Some(official) = official {
        for alternative in official.alternatives() {
            if let Some(index) = best_match(forms, &usable, alternative, false) {
                return chosen(index, Rule::OfficialReading);
            }
        }
        for alternative in official.alternatives() {
            if let Some(index) = best_match(forms, &usable, alternative, true) {
                return chosen(index, Rule::OfficialReadingNeutralTone);
            }
        }
    }
    let default_reading = generate_pinyin(hanzi);
    if !default_reading.is_empty() {
        if let Some(index) = best_match(forms, &usable, &default_reading, false) {
            return chosen(index, Rule::DefaultReading);
        }
    }
    if let Some(&index) = usable.iter().find(|&&i| !is_dominated(&forms[i])) {
        return chosen(index, Rule::NotDominated);
    }
    chosen(first, Rule::FirstForm)
}

/// Among the forms reading `reading` (by [`reading_key`]; with
/// `neutral_tones`, up to neutral tones and fewest differences first), the
/// first with the same capitalization and not dominated, relaxing those two
/// preferences in that order.
fn best_match(
    forms: &[Form],
    usable: &[usize],
    reading: &str,
    neutral_tones: bool,
) -> Option<usize> {
    let key = reading_key(reading);
    let cased: Vec<bool> = reading_key_with_case(reading)
        .chars()
        .map(char::is_uppercase)
        .collect();
    usable
        .iter()
        .filter_map(|&i| {
            let form_key = reading_key(&forms[i].transcriptions.pinyin);
            let distance = if neutral_tones {
                neutral_tone_distance(&form_key, &key)?
            } else if form_key == key {
                0
            } else {
                return None;
            };
            let form_cased: Vec<bool> = reading_key_with_case(&forms[i].transcriptions.pinyin)
                .chars()
                .map(char::is_uppercase)
                .collect();
            Some((distance, form_cased != cased, is_dominated(&forms[i]), i))
        })
        .min()
        .map(|(.., i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{official::OfficialIndex, vocabulary::Transcriptions};

    fn form(pinyin: &str, meanings: &[&str]) -> Form {
        Form {
            traditional: None,
            transcriptions: Transcriptions {
                pinyin: pinyin.into(),
            },
            meanings: meanings.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn official(hanzi: &str, pinyin: &str) -> OfficialIndex {
        OfficialIndex::parse(&format!(
            "ID,Simplified,Pinyin,Level,Variants\nL1-0001,{hanzi},{pinyin},1,\n"
        ))
        .unwrap()
    }

    /// 吧 as upstream has it: loanword first, the HSK particle second.
    fn ba_forms() -> Vec<Form> {
        vec![
            form(
                "bā",
                &[
                    "bar (loanword) (serving drinks, or providing Internet access etc)",
                    "to puff (on a pipe etc)",
                    "(onom.) bang",
                    "abbr. for 贴吧",
                ],
            ),
            form("ba", &["(modal particle indicating suggestion or surmise)"]),
            form("biā", &["(onom.) smack!"]),
        ]
    }

    #[test]
    fn penalty_meanings_and_domination() {
        for meaning in [
            "surname Huan",
            "Li (surname)",
            "old variant of 帮",
            "unofficial variant of 瞭",
            "(onom.) bang",
            "abbr. for 贴吧",
            "dozen (loanword)",
            "erhua variant of 一点",
            "Japanese variant of 樂",
            "used in 葡萄",
            "see 不见得",
        ] {
            assert!(is_penalty_meaning(meaning), "{meaning:?}");
        }
        for meaning in [
            "(used in comparison) than",
            "best in their category, e.g. see 百强",
            "to see",
            "still",
        ] {
            assert!(!is_penalty_meaning(meaning), "{meaning:?}");
        }
        assert!(is_dominated(&form("bā", &["surname Ba"])));
        assert!(is_dominated(&form("x", &[])));
        assert!(is_dominated(&form("x", &["  "])));
        assert!(
            is_dominated(&ba_forms()[0]),
            "3 of 4 meanings are penalties"
        );
        assert!(
            !is_dominated(&form("lǐ", &["surname Li", "plum"])),
            "exactly half"
        );
    }

    #[test]
    fn rule_one_picks_the_official_reading_over_the_first_form() {
        let index = official("吧", "ba");
        let selection = select_form("吧", &ba_forms(), index.first_reading("吧")).unwrap();
        assert_eq!(
            selection,
            Selection {
                index: 1,
                rule: Rule::OfficialReading
            }
        );
    }

    #[test]
    fn rule_one_prefers_the_official_capitalization_then_undominated_forms() {
        let zhong = vec![
            form("Zhōng", &["China", "Chinese", "surname Zhong"]),
            form("zhōng", &["within", "middle"]),
            form("zhòng", &["to hit (the mark)"]),
        ];
        let index = official("中", "zhōng");
        assert_eq!(
            select_form("中", &zhong, index.first_reading("中"))
                .unwrap()
                .index,
            1
        );

        let hai = vec![
            form("Huán", &["surname Huan"]),
            form("hái", &["still"]),
            form("huán", &["to pay back"]),
        ];
        let index = official("还", "huán");
        assert_eq!(
            select_form("还", &hai, index.first_reading("还"))
                .unwrap()
                .index,
            2
        );

        // Same capitalization everywhere: the undominated form wins.
        let le = vec![
            form("liǎo", &["unofficial variant of 瞭"]),
            form("liǎo", &["to finish", "to understand clearly"]),
        ];
        let index = official("了", "liǎo");
        assert_eq!(
            select_form("了", &le, index.first_reading("了"))
                .unwrap()
                .index,
            1
        );
    }

    #[test]
    fn rule_one_tries_slash_alternatives_in_order() {
        let shei = vec![form("shuí", &["who"]), form("shéi", &["who"])];
        let index = official("谁", "shéi/shuí");
        let selection = select_form("谁", &shei, index.first_reading("谁")).unwrap();
        assert_eq!(
            selection,
            Selection {
                index: 1,
                rule: Rule::OfficialReading
            }
        );
    }

    #[test]
    fn rule_one_b_accepts_neutral_tones_when_nothing_matches_exactly() {
        let choose = |hanzi: &str, official_pinyin: &str, forms: &[Form]| {
            let index = official(hanzi, official_pinyin);
            select_form(hanzi, forms, index.first_reading(hanzi)).unwrap()
        };
        let neutral = |index| Selection {
            index,
            rule: Rule::OfficialReadingNeutralTone,
        };
        let haochu = [
            form("hǎo chǔ", &["easy to get along with"]),
            form(
                "hǎo chu",
                &["benefit; advantage; merit", "also pr. [hǎo chù]"],
            ),
        ];
        assert_eq!(choose("好处", "hǎochù", &haochu), neutral(1));
        let exin = [
            form("ě xīn", &["variant of 恶心"]),
            form("ě xīn", &["nausea", "disgust"]),
            form("è xīn", &["bad habit", "vice"]),
        ];
        assert_eq!(choose("恶心", "ěxin", &exin), neutral(1));
        // Fewest neutral-tone differences first.
        let qilai = [
            form(
                "qi lai",
                &["(after a verb) indicating the beginning of an action"],
            ),
            form("qǐ lai", &["to stand up", "to get up"]),
        ];
        assert_eq!(choose("起来", "qǐlái", &qilai), neutral(1));
        // An exact match wins over a neutral-tone one that comes first.
        let difang = [form("dì fang", &["place"]), form("dì fāng", &["region"])];
        assert_eq!(
            choose("地方", "dìfāng", &difang),
            Selection {
                index: 1,
                rule: Rule::OfficialReading
            }
        );
    }

    #[test]
    fn rule_two_uses_the_default_reading_without_an_official_one() {
        // The pinyin crate reads 中 as zhōng and 长 as zhǎng.
        let zhong = vec![
            form("Zhōng", &["China", "Chinese", "surname Zhong"]),
            form("zhòng", &["to hit (the mark)"]),
            form("zhōng", &["within", "middle"]),
        ];
        assert_eq!(
            select_form("中", &zhong, None),
            Some(Selection {
                index: 2,
                rule: Rule::DefaultReading
            })
        );
        // An official reading that matches no form falls through to rule 2.
        let index = official("长", "chàng");
        let chang = vec![form("cháng", &["long"]), form("zhǎng", &["to grow"])];
        assert_eq!(
            select_form("长", &chang, index.first_reading("长")),
            Some(Selection {
                index: 1,
                rule: Rule::DefaultReading
            })
        );
    }

    #[test]
    fn rules_three_and_four_are_the_fallbacks() {
        // No official reading, and no form reads like the default `xué`.
        let forms = vec![
            form("xiào", &["surname Xiao"]),
            form("jiào", &["to teach", "used in 学校"]),
            form("xiáo", &["to imitate"]),
        ];
        assert_eq!(
            select_form("学", &forms, None),
            Some(Selection {
                index: 1,
                rule: Rule::NotDominated
            })
        );
        let all_dominated = vec![form("xiào", &["surname Xiao"]), form("jiào", &["see 教"])];
        assert_eq!(
            select_form("学", &all_dominated, None),
            Some(Selection {
                index: 0,
                rule: Rule::FirstForm
            })
        );
    }

    #[test]
    fn single_and_unusable_forms() {
        assert_eq!(
            select_form("吧", &[form("bā", &["surname Ba"])], None),
            Some(Selection {
                index: 0,
                rule: Rule::SingleForm
            })
        );
        // A form without a reading is never picked; with one usable form
        // left there is no choice to make.
        let forms = vec![form(" ", &["no reading"]), form("bā", &["surname Ba"])];
        assert_eq!(
            select_form("吧", &forms, None),
            Some(Selection {
                index: 1,
                rule: Rule::SingleForm
            })
        );
        assert_eq!(select_form("吧", &[form("", &["x"])], None), None);
        assert_eq!(select_form("吧", &[], None), None);
    }
}
