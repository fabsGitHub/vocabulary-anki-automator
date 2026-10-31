//! The official HSK 3.0 list (ivankra/hsk30, `hsk30.csv`).
//!
//! It is the only source that says which *reading* of a character is the HSK
//! word: it has one row per reading (地 is `de` in L1-0066 and `dì` in
//! L1-0069), while drkameleon's entries list every reading CC-CEDICT knows,
//! in dictionary order (吧's first form is the loanword `bā`, HSK 1 means the
//! particle `ba`). The build therefore uses this list to pick the form, and to
//! report where the two upstreams disagree about a word's level.
//!
//! Composite rows (`爸爸|爸`, `第（第二）`, `…极了`, homograph-numbered `称1`)
//! carry their parts in the JSON `Variants` column; every composite row in the
//! pinned file has it, so the index uses the variants and never has to parse
//! the composite notation itself.

use std::{collections::HashMap, path::Path};

use serde::Deserialize;

use crate::error::{Error, Result};

/// One reading of one word on the official list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialReading {
    /// `L1-0066`.
    pub id: String,
    /// 1–6, or 7 for the 7–9 band (the same encoding as `hsk_level`).
    pub level: u8,
    /// As written in the list: solid, possibly with `'`, `-` or `a/b`
    /// alternatives.
    pub pinyin: String,
    /// The traditional form(s) of this reading (`台|臺` lists two); empty when
    /// the list has none. CC-CEDICT, and so drkameleon, keeps one form per
    /// traditional headword, so this is what tells same-reading forms apart:
    /// 后 `hòu` is 後 ("behind"), not 后 ("empress").
    pub traditional: Vec<String>,
    /// `(level, number in the ID, row)`: what "lowest level, then lowest ID"
    /// sorts by.
    rank: (u8, u32, usize),
}

impl OfficialReading {
    /// The alternative readings of a slash-separated entry (`shéi/shuí`), in
    /// the list's order.
    pub fn alternatives(&self) -> impl Iterator<Item = &str> {
        self.pinyin
            .split('/')
            .map(str::trim)
            .filter(|alt| !alt.is_empty())
    }
}

/// Every official reading, by simplified hanzi.
#[derive(Debug, Clone, Default)]
pub struct OfficialIndex {
    by_hanzi: HashMap<String, Vec<OfficialReading>>,
    rows: usize,
}

#[derive(Deserialize)]
struct Variant {
    #[serde(rename = "Simplified")]
    simplified: String,
    #[serde(rename = "Pinyin")]
    pinyin: String,
    #[serde(rename = "Traditional", default)]
    traditional: Option<String>,
}

/// `台|臺` -> `["台", "臺"]`.
fn traditional_forms(text: &str) -> Vec<String> {
    text.split('|')
        .map(str::trim)
        .filter(|form| !form.is_empty())
        .map(str::to_string)
        .collect()
}

impl OfficialIndex {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|err| Error::io(path, err))?;
        Self::parse(&text).map_err(|err| match err {
            Error::Data(message) => Error::Data(format!("{}: {message}", path.display())),
            other => other,
        })
    }

    /// Parses the CSV (header `ID,Simplified,...,Pinyin,...,Level,...,Variants`).
    pub fn parse(csv: &str) -> Result<Self> {
        let records = parse_csv(csv).map_err(Error::Data)?;
        let mut records = records.into_iter();
        let header = records
            .next()
            .ok_or_else(|| Error::Data("the official list is empty".into()))?;
        let column = |name: &str| {
            header
                .iter()
                .position(|field| field.trim() == name)
                .ok_or_else(|| Error::Data(format!("the official list has no {name} column")))
        };
        let (id_col, hanzi_col, pinyin_col, level_col) = (
            column("ID")?,
            column("Simplified")?,
            column("Pinyin")?,
            column("Level")?,
        );
        let variants_col = column("Variants").ok();
        let traditional_col = column("Traditional").ok();

        let mut index = Self::default();
        for (row, record) in records.enumerate() {
            let line = row + 2;
            let field = |col: usize| {
                record.get(col).map(|s| s.trim()).ok_or_else(|| {
                    Error::Data(format!("official list line {line}: missing column {col}"))
                })
            };
            let id = field(id_col)?;
            let level_text = field(level_col)?;
            let level = parse_level(level_text).ok_or_else(|| {
                Error::Data(format!(
                    "official list line {line}: unknown level {level_text:?}"
                ))
            })?;
            let number = id
                .rsplit('-')
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(u32::MAX);
            // A composite row's own Traditional column is composite notation
            // too (`爸爸|爸`), so its parts take theirs from the variants only.
            let parts: Vec<(String, String, Vec<String>)> = match variants_col
                .and_then(|col| record.get(col))
                .map(|s| s.trim())
            {
                Some(json) if !json.is_empty() => serde_json::from_str::<Vec<Variant>>(json)
                    .map_err(|err| {
                        Error::Data(format!(
                            "official list line {line}: bad Variants JSON: {err}"
                        ))
                    })?
                    .into_iter()
                    .map(|variant| {
                        let traditional = variant
                            .traditional
                            .as_deref()
                            .map(traditional_forms)
                            .unwrap_or_default();
                        (variant.simplified, variant.pinyin, traditional)
                    })
                    .collect(),
                _ => vec![(
                    field(hanzi_col)?.to_string(),
                    field(pinyin_col)?.to_string(),
                    match traditional_col {
                        Some(col) => traditional_forms(field(col)?),
                        None => Vec::new(),
                    },
                )],
            };
            for (hanzi, pinyin, traditional) in parts {
                let hanzi = hanzi.trim();
                if hanzi.is_empty() {
                    continue;
                }
                index
                    .by_hanzi
                    .entry(hanzi.to_string())
                    .or_default()
                    .push(OfficialReading {
                        id: id.to_string(),
                        level,
                        pinyin: pinyin.trim().to_string(),
                        traditional,
                        rank: (level, number, row),
                    });
            }
            index.rows += 1;
        }
        Ok(index)
    }

    /// Data rows read (not counting the header).
    pub fn row_count(&self) -> usize {
        self.rows
    }

    pub fn readings(&self, hanzi: &str) -> &[OfficialReading] {
        self.by_hanzi.get(hanzi).map_or(&[], Vec::as_slice)
    }

    /// The reading at the word's lowest official level, the lowest ID on a
    /// tie: the reading a learner meets first.
    pub fn first_reading(&self, hanzi: &str) -> Option<&OfficialReading> {
        self.readings(hanzi)
            .iter()
            .min_by_key(|reading| reading.rank)
    }
}

/// `"1"`..`"6"` -> 1..6, `"7-9"` -> 7.
pub fn parse_level(text: &str) -> Option<u8> {
    match text.trim() {
        "7-9" => Some(7),
        other => other.parse().ok().filter(|level| (1..=6).contains(level)),
    }
}

/// RFC 4180 CSV: comma-separated fields, `"`-quoted fields that may contain
/// commas, doubled quotes and line breaks, `\n` or `\r\n` record ends. Blank
/// lines are skipped and a leading BOM is ignored.
pub fn parse_csv(text: &str) -> std::result::Result<Vec<Vec<String>>, String> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut field_started = false;
    let mut chars = text.chars().peekable();
    let mut line = 1;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => {
                    if c == '\n' {
                        line += 1;
                    }
                    field.push(c);
                }
            }
            continue;
        }
        match c {
            '"' if !field_started => {
                quoted = true;
                field_started = true;
            }
            ',' => {
                record.push(std::mem::take(&mut field));
                field_started = false;
            }
            '\r' | '\n' => {
                if c == '\r' {
                    chars.next_if_eq(&'\n');
                }
                line += 1;
                record.push(std::mem::take(&mut field));
                field_started = false;
                if !(record.len() == 1 && record[0].is_empty()) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            c => {
                field.push(c);
                field_started = true;
            }
        }
    }
    if quoted {
        return Err(format!("unterminated quoted field before line {line}"));
    }
    if field_started || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        "ID,Simplified,Traditional,Pinyin,POS,Level,WebNo,WebPinyin,OCR,Variants,CEDICT";

    #[test]
    fn csv_handles_quotes_commas_and_line_breaks() {
        let text = "a,b,c\r\n1,\"x, y\",\"say \"\"hi\"\"\"\n\n2,\"multi\nline\",\n3,,";
        assert_eq!(
            parse_csv(text).unwrap(),
            vec![
                vec!["a", "b", "c"],
                vec!["1", "x, y", "say \"hi\""],
                vec!["2", "multi\nline", ""],
                vec!["3", "", ""],
            ]
        );
        assert_eq!(
            parse_csv("\u{FEFF}h\nv\n").unwrap(),
            vec![vec!["h"], vec!["v"]]
        );
        assert!(parse_csv("a,\"open").is_err());
    }

    #[test]
    fn levels_parse_including_the_combined_band() {
        assert_eq!(parse_level("1"), Some(1));
        assert_eq!(parse_level(" 6 "), Some(6));
        assert_eq!(parse_level("7-9"), Some(7));
        for bad in ["0", "7", "10", "", "x"] {
            assert_eq!(parse_level(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn index_expands_variants_and_picks_the_lowest_level_then_id() {
        let csv = format!(
            "{HEADER}\n\
             L1-0069,地,地,dì,N,1,1755,dì,地,,地|地[di4]\n\
             L1-0066,地,地,de,Aux,1,1698,de,地,,地|地[de5]\n\
             L2-0115,得,得,dé,V,2,1679,dé,得,,得|得[de2]\n\
             L4-0172,得,得,děi,V,4,1702,děi,得,,得|得[dei3]\n\
             L1-0004,爸爸|爸,爸爸|爸,bàba|bà,N,1,83,bàba|bà,爸爸｜爸,\"[{{\"\"Simplified\"\": \"\"爸爸\"\", \"\"Pinyin\"\": \"\"bàba\"\"}}, {{\"\"Simplified\"\": \"\"爸\"\", \"\"Pinyin\"\": \"\"bà\"\"}}]\",x\n\
             L1-0325,谁,誰,shéi/shuí,Pr,1,1,shéi/shuí,谁,,x\n\
             L7-0001,阿姨,阿姨,āyí,N,7-9,1,āyí,阿姨,,x\n"
        );
        let index = OfficialIndex::parse(&csv).unwrap();
        assert_eq!(index.row_count(), 7);

        let de = index.first_reading("地").unwrap();
        assert_eq!((de.id.as_str(), de.pinyin.as_str()), ("L1-0066", "de"));
        assert_eq!(index.readings("地").len(), 2);
        assert_eq!(index.first_reading("得").unwrap().pinyin, "dé");
        assert_eq!(index.first_reading("爸爸").unwrap().pinyin, "bàba");
        assert_eq!(index.first_reading("爸").unwrap().pinyin, "bà");
        assert!(index.readings("爸爸|爸").is_empty());
        let who = index.first_reading("谁").unwrap();
        assert_eq!(who.alternatives().collect::<Vec<_>>(), ["shéi", "shuí"]);
        assert_eq!(index.first_reading("阿姨").unwrap().level, 7);
        assert!(index.first_reading("没有").is_none());
    }

    #[test]
    fn readings_carry_their_traditional_forms() {
        let csv = format!(
            "{HEADER}\n\
             L1-0213,里,裡|裏,lǐ,N,1,1,lǐ,里,,x\n\
             L1-0147,后,後,hòu,N,1,1,hòu,后,,x\n\
             L1-0224,妈妈|妈,媽媽|媽,māma|mā,N,1,1,māma|mā,妈妈｜妈,\"[{{\"\"Simplified\"\": \"\"妈妈\"\", \"\"Pinyin\"\": \"\"māma\"\", \"\"Traditional\"\": \"\"媽媽\"\"}}, {{\"\"Simplified\"\": \"\"妈\"\", \"\"Pinyin\"\": \"\"mā\"\"}}]\",x\n"
        );
        let index = OfficialIndex::parse(&csv).unwrap();
        assert_eq!(index.first_reading("里").unwrap().traditional, ["裡", "裏"]);
        assert_eq!(index.first_reading("后").unwrap().traditional, ["後"]);
        assert_eq!(index.first_reading("妈妈").unwrap().traditional, ["媽媽"]);
        assert!(
            index.first_reading("妈").unwrap().traditional.is_empty(),
            "a variant without its own form does not inherit the composite column"
        );

        let without_column = OfficialIndex::parse("ID,Simplified,Pinyin,Level\nL1-0147,后,hòu,1\n");
        assert!(without_column
            .unwrap()
            .first_reading("后")
            .unwrap()
            .traditional
            .is_empty());
    }

    #[test]
    fn index_rejects_rows_it_cannot_interpret() {
        let missing_column = "ID,Simplified,Level\nL1-0001,爱,1\n";
        assert!(matches!(
            OfficialIndex::parse(missing_column),
            Err(Error::Data(message)) if message.contains("Pinyin")
        ));
        let bad_level = format!("{HEADER}\nL1-0001,爱,愛,ài,V,one,18,ài,爱,,x\n");
        assert!(matches!(
            OfficialIndex::parse(&bad_level),
            Err(Error::Data(message)) if message.contains("line 2")
        ));
        let bad_variants = format!("{HEADER}\nL1-0001,爱,愛,ài,V,1,18,ài,爱,[oops,x\n");
        assert!(matches!(
            OfficialIndex::parse(&bad_variants),
            Err(Error::Data(message)) if message.contains("Variants")
        ));
    }
}
