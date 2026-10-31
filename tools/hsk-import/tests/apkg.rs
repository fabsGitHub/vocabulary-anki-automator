//! `apkg-enrich` against synthetic Anki packages built here with rusqlite and
//! zip: a legacy collection with the user's field names, a media map and
//! media files, plus the package shapes that must be refused.

use std::{
    io::Write,
    path::{Path, PathBuf},
};

use hsk_import::{
    apkg::{enrich_from_apkg, ApkgFields},
    cli::{run, Command},
    output::{read_curriculum, write_curriculum},
    Error,
};
use rusqlite::{params, Connection};
use serde_json::json;
use vocab_core::CurriculumItem;
use zip::{write::SimpleFileOptions, ZipWriter};

const CHINESE_MODEL: i64 = 1_700_000_000_001;
const BASIC_MODEL: i64 = 1_700_000_000_002;

/// The user's note type, with its fields listed out of `ord` order to show
/// that `ord`, not the array position, decides.
fn models() -> String {
    let field = |name: &str, ord: u32| json!({ "name": name, "ord": ord });
    json!({
        CHINESE_MODEL.to_string(): {
            "id": CHINESE_MODEL,
            "name": "Chinese (HSK 1-4)",
            "flds": [
                field("Chinese", 0),
                field("English", 2),
                field("Pinyin", 1),
                field("Sound", 3),
                field("Sentence", 4),
                field("Sentence Pinyin", 5),
                field("English Sentence", 6),
                field("Sentence Sound", 7),
                field("Image", 8),
            ]
        },
        BASIC_MODEL.to_string(): {
            "id": BASIC_MODEL,
            "name": "Basic",
            "flds": [field("Front", 0), field("Back", 1)]
        }
    })
    .to_string()
}

/// A legacy collection (Anki schema 11 tables `col` and `notes`) holding
/// `notes` as `(model id, fields)`.
fn collection(path: &Path, notes: &[(i64, Vec<&str>)]) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE col (id integer primary key, crt integer not null, mod integer not null,
             scm integer not null, ver integer not null, dty integer not null,
             usn integer not null, ls integer not null, conf text not null,
             models text not null, decks text not null, dconf text not null,
             tags text not null);
         CREATE TABLE notes (id integer primary key, guid text not null, mid integer not null,
             mod integer not null, usn integer not null, tags text not null,
             flds text not null, sfld integer not null, csum integer not null,
             flags integer not null, data text not null);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO col VALUES (1, 0, 0, 0, 11, 0, 0, 0, '{}', ?1, '{}', '{}', '{}')",
        [models()],
    )
    .unwrap();
    for (i, (mid, fields)) in notes.iter().enumerate() {
        conn.execute(
            "INSERT INTO notes VALUES (?1, ?2, ?3, 0, 0, '', ?4, ?5, 0, 0, '')",
            params![
                1_000 + i as i64,
                format!("guid{i}"),
                mid,
                fields.join("\x1f"),
                fields[0]
            ],
        )
        .unwrap();
    }
}

/// Zips `entries` (name, bytes) into `path`.
fn package(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let mut zip = ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn chinese_note(fields: [&str; 9]) -> (i64, Vec<&str>) {
    (CHINESE_MODEL, fields.to_vec())
}

/// The user's deck in miniature: a fully equipped note for 学习, a later
/// duplicate of it, notes with missing and hostile media, a word outside the
/// curriculum, and a note of another type.
fn deck(dir: &Path) -> PathBuf {
    let db = dir.join("collection.anki21");
    collection(
        &db,
        &[
            chinese_note([
                "<b>学习</b>",
                "xué xí",
                "to study",
                "[sound:xuexi.mp3]",
                "我<b>学习</b>中文。",
                "Wǒ xuéxí Zhōngwén.",
                "I study Chinese.&nbsp;",
                "[sound:xuexi_s.mp3]",
                "<img src=\"xuexi.jpg\">",
            ]),
            chinese_note([
                "学习",
                "xué xí",
                "to learn",
                "[sound:other.mp3]",
                "第二句。",
                "",
                "Second.",
                "",
                "",
            ]),
            chinese_note([
                "你好",
                "nǐ hǎo",
                "hello",
                "[sound:missing.mp3]",
                "你好！",
                "Nǐ hǎo!",
                "Hello!",
                "",
                "",
            ]),
            chinese_note([
                "银行",
                "yín háng",
                "bank",
                "[sound:../evil.mp3]",
                "",
                "",
                "",
                "",
                "<img src='a/b.jpg'>",
            ]),
            chinese_note(["不在", "bú zài", "absent", "", "", "", "", "", ""]),
            (BASIC_MODEL, vec!["front", "back"]),
        ],
    );
    let apkg = dir.join("Chinese Vocabulary (HSK 1-4).apkg");
    let media = json!({
        "0": "xuexi.mp3",
        "1": "xuexi_s.mp3",
        "2": "xuexi.jpg",
        "3": "../evil.mp3",
        "4": "other.mp3"
    });
    package(
        &apkg,
        &[
            ("collection.anki21", std::fs::read(&db).unwrap()),
            ("media", media.to_string().into_bytes()),
            ("0", b"ID3 word audio".to_vec()),
            ("1", b"ID3 sentence audio".to_vec()),
            ("2", b"\xff\xd8 image".to_vec()),
            ("3", b"must never be written".to_vec()),
            ("4", b"duplicate note audio".to_vec()),
        ],
    );
    apkg
}

fn item(hanzi: &str, order: i64) -> CurriculumItem {
    CurriculumItem {
        hanzi: hanzi.into(),
        traditional: None,
        hsk_level: 1,
        frequency: None,
        curriculum_order: order,
        pinyin: "x".into(),
        pos: None,
        meaning_en: Some("x".into()),
        meaning_de: None,
        cloze_sentence: None,
        example_sentence_source: None,
        example_sentence_pinyin: None,
        example_sentence_target: None,
        vocab_audio_path: None,
        sentence_audio_path: None,
        image_path: None,
    }
}

fn curriculum() -> Vec<CurriculumItem> {
    let mut items = vec![
        item("学习", 0),
        item("你好", 1),
        item("银行", 2),
        item("中国", 3),
    ];
    items[1].example_sentence_source = Some("你好吗？".into());
    items
}

#[test]
fn matching_notes_fill_sentences_and_media_and_the_rest_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let apkg = deck(dir.path());
    let media = dir.path().join("media");
    let mut items = curriculum();

    let report = enrich_from_apkg(&apkg, &mut items, &media, &ApkgFields::default()).unwrap();

    let xuexi = &items[0];
    assert_eq!(
        xuexi.example_sentence_source.as_deref(),
        Some("我学习中文。")
    );
    assert_eq!(
        xuexi.example_sentence_pinyin.as_deref(),
        Some("Wǒ xuéxí Zhōngwén.")
    );
    assert_eq!(
        xuexi.example_sentence_target.as_deref(),
        Some("I study Chinese.")
    );
    assert_eq!(xuexi.vocab_audio_path.as_deref(), Some("xuexi.mp3"));
    assert_eq!(xuexi.sentence_audio_path.as_deref(), Some("xuexi_s.mp3"));
    assert_eq!(xuexi.image_path.as_deref(), Some("xuexi.jpg"));
    assert_eq!(
        std::fs::read(media.join("xuexi.mp3")).unwrap(),
        b"ID3 word audio"
    );
    assert_eq!(
        std::fs::read(media.join("xuexi_s.mp3")).unwrap(),
        b"ID3 sentence audio"
    );
    assert_eq!(
        std::fs::read(media.join("xuexi.jpg")).unwrap(),
        b"\xff\xd8 image"
    );

    // An existing sentence is kept; the rest of the note still fills in.
    let nihao = &items[1];
    assert_eq!(nihao.example_sentence_source.as_deref(), Some("你好吗？"));
    assert_eq!(nihao.example_sentence_pinyin.as_deref(), Some("Nǐ hǎo!"));
    assert_eq!(
        nihao.vocab_audio_path, None,
        "its audio is not in the package"
    );

    let yinhang = &items[2];
    assert_eq!(
        (
            yinhang.vocab_audio_path.as_deref(),
            yinhang.image_path.as_deref()
        ),
        (None, None)
    );
    assert_eq!(
        items[3],
        curriculum()[3],
        "a word the deck lacks is untouched"
    );

    let mut written: Vec<String> = std::fs::read_dir(&media)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    written.sort();
    assert_eq!(written, ["xuexi.jpg", "xuexi.mp3", "xuexi_s.mp3"]);
    assert!(!dir.path().join("evil.mp3").exists());

    assert_eq!(report.collection, "collection.anki21");
    assert_eq!(report.notes, 6);
    assert_eq!(report.matched, 3);
    assert_eq!(report.duplicates, 1, "the second 学习 note is ignored");
    assert_eq!(report.unmatched, ["不在"]);
    assert_eq!(report.without_word_field, 1);
    assert_eq!(
        (
            report.sentences,
            report.sentence_pinyin,
            report.sentence_translations
        ),
        (1, 2, 2)
    );
    assert_eq!(
        (report.vocab_audio, report.sentence_audio, report.images),
        (1, 1, 1)
    );
    assert_eq!(report.media_missing, ["missing.mp3"]);
    assert_eq!(report.media_rejected, ["../evil.mp3", "a/b.jpg"]);
}

#[test]
fn rerunning_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let apkg = deck(dir.path());
    let media = dir.path().join("media");
    let mut items = curriculum();
    enrich_from_apkg(&apkg, &mut items, &media, &ApkgFields::default()).unwrap();
    let once = items.clone();

    let report = enrich_from_apkg(&apkg, &mut items, &media, &ApkgFields::default()).unwrap();

    assert_eq!(items, once);
    assert_eq!(report.matched, 3);
    assert_eq!(
        (report.sentences, report.vocab_audio, report.images),
        (0, 0, 0)
    );
}

#[test]
fn field_names_are_configurable() {
    let dir = tempfile::tempdir().unwrap();
    let apkg = deck(dir.path());
    let mut items = vec![item("front", 0)];
    let fields = ApkgFields {
        word: "front".into(),
        sentence: "Back".into(),
        ..ApkgFields::default()
    };
    // Field names match case-insensitively when there is no exact match.
    let report = enrich_from_apkg(&apkg, &mut items, &dir.path().join("m"), &fields).unwrap();
    assert_eq!(report.matched, 1);
    assert_eq!(items[0].example_sentence_source.as_deref(), Some("back"));
}

#[test]
fn new_format_packages_are_refused_with_the_reexport_hint() {
    let dir = tempfile::tempdir().unwrap();
    let apkg = dir.path().join("new.apkg");
    let stub = dir.path().join("stub.anki2");
    collection(&stub, &[]);
    package(
        &apkg,
        &[
            ("collection.anki2", std::fs::read(&stub).unwrap()),
            ("collection.anki21b", b"\x28\xb5\x2f\xfd zstd".to_vec()),
            ("media", b"\x28\xb5\x2f\xfd".to_vec()),
        ],
    );
    let mut items = curriculum();

    let result = enrich_from_apkg(
        &apkg,
        &mut items,
        &dir.path().join("m"),
        &ApkgFields::default(),
    );

    match result {
        Err(Error::Apkg(message)) => {
            assert!(message.contains("collection.anki21b"), "{message}");
            assert!(message.contains("Support older Anki versions"), "{message}");
        }
        other => panic!("expected an apkg error, got {other:?}"),
    }
    assert_eq!(items, curriculum());
}

#[test]
fn the_real_collection_wins_over_the_anki2_stub_and_anki2_alone_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.anki21");
    collection(
        &real,
        &[chinese_note(["学习", "", "", "", "真的。", "", "", "", ""])],
    );
    let stub = dir.path().join("stub.anki2");
    collection(
        &stub,
        &[chinese_note([
            "学习",
            "",
            "",
            "",
            "Please update Anki.",
            "",
            "",
            "",
            "",
        ])],
    );

    let both = dir.path().join("both.apkg");
    package(
        &both,
        &[
            ("collection.anki2", std::fs::read(&stub).unwrap()),
            ("collection.anki21", std::fs::read(&real).unwrap()),
        ],
    );
    let mut items = curriculum();
    let report = enrich_from_apkg(
        &both,
        &mut items,
        &dir.path().join("m"),
        &ApkgFields::default(),
    )
    .unwrap();
    assert_eq!(report.collection, "collection.anki21");
    assert_eq!(items[0].example_sentence_source.as_deref(), Some("真的。"));

    let old = dir.path().join("old.apkg");
    package(&old, &[("collection.anki2", std::fs::read(&real).unwrap())]);
    let mut items = curriculum();
    let report = enrich_from_apkg(
        &old,
        &mut items,
        &dir.path().join("m"),
        &ApkgFields::default(),
    )
    .unwrap();
    assert_eq!(report.collection, "collection.anki2");
    assert_eq!(
        report.media_missing,
        Vec::<String>::new(),
        "no media map, no media"
    );
    assert_eq!(items[0].example_sentence_source.as_deref(), Some("真的。"));
}

#[test]
fn files_that_are_not_anki_packages_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut items = curriculum();
    let fields = ApkgFields::default();
    let media = dir.path().join("m");

    let empty_zip = dir.path().join("empty.apkg");
    package(&empty_zip, &[("readme.txt", b"hi".to_vec())]);
    assert!(matches!(
        enrich_from_apkg(&empty_zip, &mut items, &media, &fields),
        Err(Error::Apkg(message)) if message.contains("not an Anki deck package")
    ));

    let not_zip = dir.path().join("text.apkg");
    std::fs::write(&not_zip, "plain text").unwrap();
    assert!(matches!(
        enrich_from_apkg(&not_zip, &mut items, &media, &fields),
        Err(Error::Zip(_))
    ));
    assert!(matches!(
        enrich_from_apkg(&dir.path().join("absent.apkg"), &mut items, &media, &fields),
        Err(Error::Io { .. })
    ));
}

#[test]
fn the_cli_writes_the_enriched_curriculum_back_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let apkg = deck(dir.path());
    let path = dir.path().join("hsk_curriculum.json");
    write_curriculum(&path, &curriculum()).unwrap();

    run(Command::ApkgEnrich {
        apkg,
        curriculum: path.clone(),
        media_out: dir.path().join("media"),
        out: None,
        fields: ApkgFields::default(),
    })
    .unwrap();

    let items = read_curriculum(&path).unwrap();
    assert_eq!(items[0].vocab_audio_path.as_deref(), Some("xuexi.mp3"));
    assert_eq!(
        items[0].example_sentence_source.as_deref(),
        Some("我学习中文。")
    );
}
