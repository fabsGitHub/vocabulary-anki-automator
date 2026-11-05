//! The `hsk_curriculum` table: bundled reference data, replaced as a whole.
//!
//! Both apps call [`Store::ensure_curriculum_seeded`] at startup with the
//! curriculum compiled into their binary. Rewriting some 11,000 rows on every
//! launch would be wasteful, so the table records a fingerprint of what it
//! holds in `app_state` and is rewritten only when the caller's data differs.
//! (If the two apps ship different curriculum revisions, each launch of the
//! other app reseeds once more; that is correct, just not free.)
//!
//! The fingerprint is a hand-written FNV-1a 64: it is persisted, so it must
//! not change between Rust releases the way `std`'s `DefaultHasher` may.

use rusqlite::{named_params, Row};

use super::{Store, StoreResult};
use crate::model::CurriculumItem;

/// `app_state` key holding [`curriculum_fingerprint`] of the rows in
/// `hsk_curriculum`.
pub const CURRICULUM_FINGERPRINT_KEY: &str = "curriculum_fingerprint";

const SELECT_CURRICULUM: &str = "SELECT hanzi, traditional, hsk_level, frequency, \
     curriculum_order, pinyin, pos, meaning_en, meaning_de, cloze_sentence, \
     example_sentence_source, example_sentence_pinyin, example_sentence_target, \
     vocab_audio_path, sentence_audio_path, image_path FROM hsk_curriculum";

const INSERT_CURRICULUM_ITEM: &str = "INSERT INTO hsk_curriculum (hanzi, traditional, \
     hsk_level, frequency, curriculum_order, pinyin, pos, meaning_en, meaning_de, \
     cloze_sentence, example_sentence_source, example_sentence_pinyin, \
     example_sentence_target, vocab_audio_path, sentence_audio_path, image_path) VALUES \
     (:hanzi, :traditional, :hsk_level, :frequency, :curriculum_order, :pinyin, :pos, \
     :meaning_en, :meaning_de, :cloze_sentence, :example_sentence_source, \
     :example_sentence_pinyin, :example_sentence_target, :vocab_audio_path, \
     :sentence_audio_path, :image_path)";

impl Store {
    pub fn curriculum_count(&self) -> StoreResult<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM hsk_curriculum", [], |row| row.get(0))?)
    }

    /// Replaces the whole curriculum with `items` in one transaction and
    /// records their fingerprint. If any item is rejected (a duplicate
    /// `hanzi` or `curriculum_order`), nothing changes.
    pub fn replace_curriculum(&mut self, items: &[CurriculumItem]) -> StoreResult<()> {
        let fingerprint = curriculum_fingerprint(items);
        self.with_transaction(|store| store.write_curriculum(items, &fingerprint))
    }

    /// Makes `hsk_curriculum` hold exactly `items`, rewriting it only when
    /// the stored fingerprint differs (or the row count does, after a manual
    /// edit). Returns whether it wrote. The check and the write share one
    /// transaction, so two apps starting together seed once.
    pub fn ensure_curriculum_seeded(&mut self, items: &[CurriculumItem]) -> StoreResult<bool> {
        let fingerprint = curriculum_fingerprint(items);
        self.with_transaction(|store| {
            let seeded = store.get_state(CURRICULUM_FINGERPRINT_KEY)?.as_deref()
                == Some(fingerprint.as_str())
                && store.curriculum_count()? == items.len() as i64;
            if seeded {
                return Ok(false);
            }
            store.write_curriculum(items, &fingerprint)?;
            Ok(true)
        })
    }

    /// The entry for exactly this hanzi. Unlike `CurriculumIndex::lookup`,
    /// the key is not trimmed.
    pub fn get_curriculum_item(&self, hanzi: &str) -> StoreResult<Option<CurriculumItem>> {
        let mut statement = self
            .conn()
            .prepare_cached(&format!("{SELECT_CURRICULUM} WHERE hanzi = ?1"))?;
        let mut items = statement.query_map([hanzi], curriculum_item_from_row)?;
        Ok(items.next().transpose()?)
    }

    /// The whole curriculum by `curriculum_order`: the order in which new
    /// items are introduced.
    pub fn curriculum_in_order(&self) -> StoreResult<Vec<CurriculumItem>> {
        let mut statement = self.conn().prepare_cached(&format!(
            "{SELECT_CURRICULUM} ORDER BY curriculum_order ASC"
        ))?;
        let items = statement.query_map([], curriculum_item_from_row)?;
        Ok(items.collect::<Result<_, _>>()?)
    }

    /// Must run inside a transaction: on its own, a failure halfway would
    /// leave a partial curriculum behind.
    fn write_curriculum(&self, items: &[CurriculumItem], fingerprint: &str) -> StoreResult<()> {
        self.conn().execute("DELETE FROM hsk_curriculum", [])?;
        let mut insert = self.conn().prepare(INSERT_CURRICULUM_ITEM)?;
        for item in items {
            insert.execute(named_params! {
                ":hanzi": item.hanzi,
                ":traditional": item.traditional,
                ":hsk_level": item.hsk_level,
                ":frequency": item.frequency,
                ":curriculum_order": item.curriculum_order,
                ":pinyin": item.pinyin,
                ":pos": item.pos,
                ":meaning_en": item.meaning_en,
                ":meaning_de": item.meaning_de,
                ":cloze_sentence": item.cloze_sentence,
                ":example_sentence_source": item.example_sentence_source,
                ":example_sentence_pinyin": item.example_sentence_pinyin,
                ":example_sentence_target": item.example_sentence_target,
                ":vocab_audio_path": item.vocab_audio_path,
                ":sentence_audio_path": item.sentence_audio_path,
                ":image_path": item.image_path,
            })?;
        }
        self.set_state(CURRICULUM_FINGERPRINT_KEY, fingerprint)
    }
}

fn curriculum_item_from_row(row: &Row<'_>) -> rusqlite::Result<CurriculumItem> {
    Ok(CurriculumItem {
        hanzi: row.get("hanzi")?,
        traditional: row.get("traditional")?,
        hsk_level: row.get("hsk_level")?,
        frequency: row.get("frequency")?,
        curriculum_order: row.get("curriculum_order")?,
        pinyin: row.get("pinyin")?,
        pos: row.get("pos")?,
        meaning_en: row.get("meaning_en")?,
        meaning_de: row.get("meaning_de")?,
        cloze_sentence: row.get("cloze_sentence")?,
        example_sentence_source: row.get("example_sentence_source")?,
        example_sentence_pinyin: row.get("example_sentence_pinyin")?,
        example_sentence_target: row.get("example_sentence_target")?,
        vocab_audio_path: row.get("vocab_audio_path")?,
        sentence_audio_path: row.get("sentence_audio_path")?,
        image_path: row.get("image_path")?,
    })
}

/// Fingerprint of a curriculum: FNV-1a 64 over every field of every item,
/// as 16 lowercase hex digits.
///
/// Items are visited in `curriculum_order` (ties by `hanzi`), so the same
/// curriculum in any order has the same fingerprint, and any edit to any
/// field changes it. Strings are length-prefixed and `None` is distinct from
/// `Some("")`, so moving text from one field into its neighbour changes the
/// fingerprint too.
pub fn curriculum_fingerprint(items: &[CurriculumItem]) -> String {
    let mut ordered: Vec<&CurriculumItem> = items.iter().collect();
    ordered.sort_by(|a, b| {
        a.curriculum_order
            .cmp(&b.curriculum_order)
            .then_with(|| a.hanzi.cmp(&b.hanzi))
    });
    let mut hash = Fnv1a64::new();
    for item in ordered {
        hash_item(&mut hash, item);
    }
    format!("{:016x}", hash.finish())
}

fn hash_item(hash: &mut Fnv1a64, item: &CurriculumItem) {
    // Destructured without `..`, so a new `CurriculumItem` field does not
    // compile until it is hashed too.
    let CurriculumItem {
        hanzi,
        traditional,
        hsk_level,
        frequency,
        curriculum_order,
        pinyin,
        pos,
        meaning_en,
        meaning_de,
        cloze_sentence,
        example_sentence_source,
        example_sentence_pinyin,
        example_sentence_target,
        vocab_audio_path,
        sentence_audio_path,
        image_path,
    } = item;
    hash.write_str(hanzi);
    hash.write_opt_str(traditional.as_deref());
    hash.write(&[*hsk_level]);
    hash.write_opt_i64(*frequency);
    hash.write_i64(*curriculum_order);
    hash.write_str(pinyin);
    for text in [
        pos,
        meaning_en,
        meaning_de,
        cloze_sentence,
        example_sentence_source,
        example_sentence_pinyin,
        example_sentence_target,
        vocab_audio_path,
        sentence_audio_path,
        image_path,
    ] {
        hash.write_opt_str(text.as_deref());
    }
}

/// 64-bit FNV-1a (<http://www.isthe.com/chongo/tech/comp/fnv/>).
struct Fnv1a64(u64);

impl Fnv1a64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    fn write_str(&mut self, text: &str) {
        self.write(&(text.len() as u64).to_le_bytes());
        self.write(text.as_bytes());
    }

    fn write_opt_str(&mut self, text: Option<&str>) {
        match text {
            None => self.write(&[0]),
            Some(text) => {
                self.write(&[1]);
                self.write_str(text);
            }
        }
    }

    fn write_opt_i64(&mut self, value: Option<i64>) {
        match value {
            None => self.write(&[0]),
            Some(value) => {
                self.write(&[1]);
                self.write_i64(value);
            }
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(hanzi: &str, order: i64) -> CurriculumItem {
        CurriculumItem {
            hanzi: hanzi.into(),
            traditional: None,
            hsk_level: 1,
            frequency: Some(100),
            curriculum_order: order,
            pinyin: "xué xí".into(),
            pos: Some("v".into()),
            meaning_en: Some("to study".into()),
            meaning_de: Some("lernen".into()),
            cloze_sentence: None,
            example_sentence_source: Some("我学习中文。".into()),
            example_sentence_pinyin: Some("wǒ xué xí zhōng wén".into()),
            example_sentence_target: Some("Ich lerne Chinesisch.".into()),
            vocab_audio_path: None,
            sentence_audio_path: None,
            image_path: None,
        }
    }

    #[test]
    fn fnv1a64_matches_the_reference_vectors() {
        let cases: [(&[u8], u64); 4] = [
            (b"", 0xcbf2_9ce4_8422_2325),
            (b"a", 0xaf63_dc4c_8601_ec8c),
            (b"foobar", 0x8594_4171_f739_67e8),
            ("学习".as_bytes(), 0xc920_f41a_c9e9_b89c),
        ];
        for (input, expected) in cases {
            let mut hash = Fnv1a64::new();
            hash.write(input);
            assert_eq!(hash.finish(), expected, "{input:?}");
        }
    }

    #[test]
    fn fingerprint_is_sixteen_hex_digits_and_ignores_input_order() {
        let items = [item("学习", 2), item("你好", 1), item("中文", 3)];
        let fingerprint = curriculum_fingerprint(&items);
        assert_eq!(fingerprint.len(), 16);
        assert!(fingerprint
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let reversed: Vec<_> = items.iter().rev().cloned().collect();
        assert_eq!(curriculum_fingerprint(&reversed), fingerprint);
        assert_ne!(curriculum_fingerprint(&items[..2]), fingerprint);
        assert_ne!(curriculum_fingerprint(&[]), fingerprint);
    }

    #[test]
    fn fingerprint_changes_when_any_field_changes() {
        type Edit = fn(&mut CurriculumItem);
        let edits: [(&str, Edit); 17] = [
            ("hanzi", |i| i.hanzi.push('们')),
            ("traditional", |i| i.traditional = Some("學習".into())),
            ("hsk_level", |i| i.hsk_level = 2),
            ("frequency", |i| i.frequency = Some(101)),
            ("frequency none", |i| i.frequency = None),
            ("curriculum_order", |i| i.curriculum_order = 7),
            ("pinyin", |i| i.pinyin = "xuéxí".into()),
            ("pos", |i| i.pos = None),
            ("meaning_en", |i| i.meaning_en = Some("to learn".into())),
            ("meaning_de", |i| i.meaning_de = Some("studieren".into())),
            ("cloze_sentence", |i| {
                i.cloze_sentence = Some("我__中文。".into())
            }),
            ("example_sentence_source", |i| {
                i.example_sentence_source = None
            }),
            ("example_sentence_pinyin", |i| {
                i.example_sentence_pinyin = None
            }),
            ("example_sentence_target", |i| {
                i.example_sentence_target = None
            }),
            ("vocab_audio_path", |i| {
                i.vocab_audio_path = Some("a.mp3".into())
            }),
            ("sentence_audio_path", |i| {
                i.sentence_audio_path = Some("s.mp3".into())
            }),
            ("image_path", |i| i.image_path = Some("i.png".into())),
        ];
        let original = curriculum_fingerprint(&[item("学习", 1)]);
        for (field, edit) in edits {
            let mut changed = item("学习", 1);
            edit(&mut changed);
            assert_ne!(
                curriculum_fingerprint(&[changed]),
                original,
                "editing {field} must change the fingerprint"
            );
        }
    }

    #[test]
    fn fingerprint_keeps_field_boundaries_and_none_apart_from_empty() {
        let mut split_early = item("学习", 1);
        split_early.meaning_en = Some("to".into());
        split_early.meaning_de = Some(" study".into());
        let mut split_late = item("学习", 1);
        split_late.meaning_en = Some("to stu".into());
        split_late.meaning_de = Some("dy".into());
        assert_ne!(
            curriculum_fingerprint(&[split_early]),
            curriculum_fingerprint(&[split_late])
        );

        let mut empty = item("学习", 1);
        empty.cloze_sentence = Some(String::new());
        assert_ne!(
            curriculum_fingerprint(&[empty]),
            curriculum_fingerprint(&[item("学习", 1)])
        );
    }
}
