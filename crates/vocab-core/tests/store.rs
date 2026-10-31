//! Integration tests for `vocab_core::store` (docs/architecture.md § 6).
//!
//! File-backed cases use a fresh temporary directory each. Where a test has
//! to see exactly what is on disk, or forge a row the store would never
//! write, it opens a plain `rusqlite` connection next to the store.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{mpsc, Arc, Barrier},
    thread,
    time::Duration as StdDuration,
};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use rusqlite::Connection;
use tempfile::TempDir;
use vocab_core::{
    store::{
        curriculum_fingerprint, DailySessionRow, ReviewLogRow, SrsStateRow, Store, StoreError,
        StoreResult, CURRICULUM_FINGERPRINT_KEY, SCHEMA_VERSION,
    },
    CaptureOrigin, Card, CurriculumItem, ScriptMetadata, ScriptVariant, StoredCapture,
};

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("valid test timestamp")
        .with_timezone(&Utc)
}

fn day(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("valid test date")
}

/// A database file in its own temporary directory, deleted on drop.
struct TempDb {
    _dir: TempDir,
    path: PathBuf,
}

impl TempDb {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("vocab.sqlite3");
        Self { _dir: dir, path }
    }

    fn open(&self) -> Store {
        Store::open(&self.path).expect("open store")
    }

    /// A connection without any of the store's setup.
    fn raw(&self) -> Connection {
        Connection::open(&self.path).expect("open raw connection")
    }
}

fn english(front: &str, captured_at: &str) -> Card {
    Card::new(front, format!("{front} (de)"), "en", "de", at(captured_at))
}

fn chinese(hanzi: &str, pinyin: &str, hsk_level: Option<u8>, captured_at: &str) -> Card {
    let mut card = Card::new(hanzi, "Bedeutung", "zh-CN", "de", at(captured_at));
    card.script_metadata = ScriptMetadata::Chinese {
        pinyin: pinyin.into(),
        hsk_level,
        script_variant: ScriptVariant::Simplified,
    };
    card
}

fn insert(store: &Store, card: &Card) -> i64 {
    store
        .insert_capture(card, CaptureOrigin::Capture, false)
        .expect("insert capture")
}

fn ids(captures: &[StoredCapture]) -> Vec<i64> {
    captures.iter().map(|capture| capture.id).collect()
}

fn fronts(store: &Store) -> Vec<String> {
    store
        .list_recent_captures(100)
        .expect("list captures")
        .into_iter()
        .map(|capture| capture.card.front)
        .collect()
}

fn new_state(card_id: i64, due: &str) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: None,
        difficulty: None,
        interval_days: 0,
        repetitions: 0,
        lapses: 0,
        fsrs_state: "new".into(),
        elapsed_days: 0,
        due_date: at(due),
        last_reviewed_at: None,
        last_rating: None,
    }
}

fn review(card_id: i64, rating: u8, reviewed_at: &str) -> ReviewLogRow {
    ReviewLogRow {
        id: 0,
        card_id,
        rating,
        reviewed_at: at(reviewed_at),
        session_date: day("2026-10-31"),
        state_before: "learning".into(),
        scheduled_days: 1,
        elapsed_days: 0,
    }
}

fn session(date: &str, completed_at: Option<&str>) -> DailySessionRow {
    DailySessionRow {
        session_date: day(date),
        requested_new_count: 5,
        reviews_due_count: 12,
        reviews_completed: 0,
        new_items_completed: 0,
        completed_at: completed_at.map(at),
    }
}

fn curriculum_item(hanzi: &str, order: i64) -> CurriculumItem {
    CurriculumItem {
        hanzi: hanzi.into(),
        traditional: None,
        hsk_level: 1,
        frequency: None,
        curriculum_order: order,
        pinyin: format!("pinyin of {hanzi}"),
        pos: None,
        meaning_en: None,
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

mod schema {
    use super::*;

    /// Every table and column of § 6.1, in declaration order (v2 columns
    /// follow v1's because `ALTER TABLE ... ADD COLUMN` appends).
    const EXPECTED_COLUMNS: [(&str, &[&str]); 8] = [
        (
            "captures",
            &[
                "id",
                "front",
                "back",
                "example_source",
                "example_target",
                "notes",
                "tags",
                "source_lang",
                "target_lang",
                "captured_at",
                "synced_to_anki",
                "pinyin",
                "hsk_level",
                "script_kind",
                "script_variant",
                "origin",
                "audio_front",
            ],
        ),
        (
            "srs_state",
            &[
                "card_id",
                "algorithm",
                "stability",
                "difficulty",
                "interval_days",
                "repetitions",
                "lapses",
                "fsrs_state",
                "elapsed_days",
                "due_date",
                "last_reviewed_at",
                "last_rating",
            ],
        ),
        (
            "hsk_curriculum",
            &[
                "hanzi",
                "traditional",
                "hsk_level",
                "frequency",
                "curriculum_order",
                "pinyin",
                "pos",
                "meaning_en",
                "meaning_de",
                "cloze_sentence",
                "example_sentence_source",
                "example_sentence_pinyin",
                "example_sentence_target",
                "vocab_audio_path",
                "sentence_audio_path",
                "image_path",
            ],
        ),
        (
            "daily_sessions",
            &[
                "session_date",
                "requested_new_count",
                "reviews_due_count",
                "reviews_completed",
                "new_items_completed",
                "completed_at",
            ],
        ),
        ("app_state", &["key", "value"]),
        (
            "review_log",
            &[
                "id",
                "card_id",
                "rating",
                "reviewed_at",
                "session_date",
                "state_before",
                "scheduled_days",
                "elapsed_days",
            ],
        ),
        (
            "xp_ledger",
            &["id", "session_date", "amount", "reason", "created_at"],
        ),
        ("streak_freezes", &["freeze_date"]),
    ];

    const EXPECTED_INDEXES: [&str; 5] = [
        "idx_captures_captured_at",
        "idx_captures_script_front",
        "idx_srs_state_due",
        "idx_review_log_card",
        "idx_xp_ledger_date",
    ];

    /// The v1 schema as Milestone F shipped it (§ 6.1), written out here
    /// instead of borrowed from the store so the upgrade of a real v1 file is
    /// what gets tested.
    const V1_SQL: &str = "
        CREATE TABLE captures (
          id              INTEGER PRIMARY KEY AUTOINCREMENT,
          front           TEXT NOT NULL,
          back            TEXT NOT NULL,
          example_source  TEXT NOT NULL DEFAULT '',
          example_target  TEXT NOT NULL DEFAULT '',
          notes           TEXT NOT NULL DEFAULT '',
          tags            TEXT NOT NULL DEFAULT '[]',
          source_lang     TEXT NOT NULL,
          target_lang     TEXT NOT NULL,
          captured_at     TEXT NOT NULL,
          synced_to_anki  INTEGER NOT NULL DEFAULT 0 CHECK (synced_to_anki IN (0, 1))
        );
        CREATE INDEX idx_captures_captured_at ON captures(captured_at);
    ";

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn journal_mode(conn: &Connection) -> String {
        conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap()
    }

    fn column_names(conn: &Connection, table: &str) -> Vec<String> {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let names = statement
            .query_map([], |row| row.get::<_, String>("name"))
            .unwrap();
        names.collect::<Result<_, _>>().unwrap()
    }

    fn names_of(conn: &Connection, kind: &str) -> Vec<String> {
        let mut statement = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = ?1 AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .unwrap();
        let names = statement.query_map([kind], |row| row.get(0)).unwrap();
        names.collect::<Result<_, _>>().unwrap()
    }

    fn schema_snapshot(conn: &Connection) -> Vec<(String, String, Option<String>)> {
        let mut statement = conn
            .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
            .unwrap();
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    }

    #[test]
    fn a_fresh_database_gets_every_table_column_and_index_of_version_3() {
        let db = TempDb::new();
        drop(db.open());
        let raw = db.raw();

        assert_eq!(SCHEMA_VERSION, 3);
        assert_eq!(user_version(&raw), 3);
        for (table, columns) in EXPECTED_COLUMNS {
            assert_eq!(column_names(&raw, table), columns, "columns of {table}");
        }
        let mut tables: Vec<&str> = EXPECTED_COLUMNS.iter().map(|(table, _)| *table).collect();
        tables.sort_unstable();
        assert_eq!(names_of(&raw, "table"), tables);
        let indexes = names_of(&raw, "index");
        for index in EXPECTED_INDEXES {
            assert!(indexes.iter().any(|name| name == index), "missing {index}");
        }
        assert_eq!(journal_mode(&raw), "wal");
    }

    #[test]
    fn open_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("Application Support/VocabularyAutomator/vocab.sqlite3");
        let store = Store::open(&path).unwrap();
        insert(&store, &english("Haus", "2026-10-31T12:00:00.000Z"));
        assert!(path.is_file());
    }

    #[test]
    fn reopening_an_up_to_date_database_changes_nothing() {
        let db = TempDb::new();
        let id = insert(&db.open(), &english("Haus", "2026-10-31T12:00:00.000Z"));
        let before = schema_snapshot(&db.raw());

        for _ in 0..2 {
            let store = db.open();
            let front = store
                .get_capture(id)
                .unwrap()
                .map(|capture| capture.card.front);
            assert_eq!(front.as_deref(), Some("Haus"));
        }

        let raw = db.raw();
        assert_eq!(schema_snapshot(&raw), before);
        assert_eq!(user_version(&raw), 3);
        let rows: i64 = raw
            .query_row("SELECT COUNT(*) FROM captures", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn a_v1_database_migrates_to_v3_and_keeps_its_captures() {
        let db = TempDb::new();
        {
            let raw = db.raw();
            raw.execute_batch(V1_SQL).unwrap();
            raw.execute(
                "INSERT INTO captures (front, back, example_source, example_target, notes, \
                 tags, source_lang, target_lang, captured_at, synced_to_anki) VALUES \
                 ('serendipity', 'glücklicher Zufall', 'It was pure serendipity.', \
                 'Es war reiner Zufall.', 'Rare word.', '[\"travel\",\"b1\"]', 'en', 'de', \
                 '2026-10-30T08:15:00.250Z', 1)",
                [],
            )
            .unwrap();
            raw.pragma_update(None, "user_version", 1).unwrap();
        }

        let store = db.open();

        let mut card = Card::new(
            "serendipity",
            "glücklicher Zufall",
            "en",
            "de",
            at("2026-10-30T08:15:00.250Z"),
        );
        card.example_source = "It was pure serendipity.".into();
        card.example_target = "Es war reiner Zufall.".into();
        card.notes = "Rare word.".into();
        card.tags = vec!["travel".into(), "b1".into()];
        assert_eq!(
            store.get_capture(1).unwrap(),
            Some(StoredCapture {
                id: 1,
                card,
                origin: CaptureOrigin::Capture,
                synced_to_anki: true,
            })
        );

        let raw = db.raw();
        assert_eq!(user_version(&raw), 3);
        let defaults: (String, String, Option<String>, Option<i64>, Option<Vec<u8>>) = raw
            .query_row(
                "SELECT script_kind, origin, pinyin, hsk_level, audio_front FROM captures \
                 WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            defaults,
            ("none".into(), "capture".into(), None, None, None)
        );

        // The v2 and v3 tables work with the migrated row, and new captures
        // continue its id sequence.
        store
            .upsert_srs_state(&new_state(1, "2026-10-31T00:00:00.000Z"))
            .unwrap();
        store
            .insert_review_log(&review(1, 3, "2026-10-31T09:00:00.000Z"))
            .unwrap();
        assert_eq!(store.count_srs_states().unwrap(), 1);
        assert_eq!(
            insert(&store, &english("Haus", "2026-10-31T12:00:00.000Z")),
            2
        );
    }

    #[test]
    fn an_unknown_schema_version_is_refused_and_the_file_left_untouched() {
        // The first version past this build, a much newer one, and one no
        // build writes.
        for version in [i64::from(SCHEMA_VERSION) + 1, 99, -1] {
            let db = TempDb::new();
            db.raw()
                .pragma_update(None, "user_version", version)
                .unwrap();

            let err = Store::open(&db.path).unwrap_err();
            assert!(
                matches!(&err, StoreError::Invalid(message) if message.contains(&version.to_string())),
                "version {version}: {err:?}"
            );

            let raw = db.raw();
            assert_eq!(user_version(&raw), version);
            assert!(names_of(&raw, "table").is_empty(), "no migration may run");
            assert_eq!(journal_mode(&raw), "delete");
        }
    }

    #[test]
    fn a_failing_migration_step_leaves_the_file_at_the_previous_version() {
        let db = TempDb::new();
        {
            let raw = db.raw();
            raw.execute_batch(V1_SQL).unwrap();
            raw.execute(
                "INSERT INTO captures (front, back, source_lang, target_lang, captured_at) \
                 VALUES ('Haus', 'house', 'de', 'en', '2026-10-31T12:00:00.000Z')",
                [],
            )
            .unwrap();
            // In the way of v2's CREATE TABLE, which runs after v2's ALTER
            // TABLE statements have already added columns to `captures`.
            raw.execute_batch("CREATE TABLE srs_state (card_id INTEGER)")
                .unwrap();
            raw.pragma_update(None, "user_version", 1).unwrap();
        }
        let v1_columns = column_names(&db.raw(), "captures");

        let err = Store::open(&db.path).unwrap_err();
        assert!(matches!(err, StoreError::Sqlite(_)), "{err:?}");

        let raw = db.raw();
        assert_eq!(user_version(&raw), 1);
        assert_eq!(
            column_names(&raw, "captures"),
            v1_columns,
            "the ALTER TABLEs of the failed step were rolled back"
        );
        // Nothing half-applied is left to trip the next attempt.
        raw.execute_batch("DROP TABLE srs_state").unwrap();
        let store = db.open();
        assert_eq!(user_version(&raw), 3);
        assert_eq!(fronts(&store), ["Haus"]);
    }

    #[test]
    fn opening_a_fresh_file_outlasts_another_connections_write_lock() {
        // Switching a fresh file to WAL upgrades a read lock to the write
        // lock. While another connection holds the write lock, SQLite refuses
        // that upgrade at once instead of waiting (waiting could deadlock),
        // so `open` has to retry rather than rely on the busy timeout.
        let db = TempDb::new();
        let holder = db.raw();
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        let release = thread::spawn(move || {
            thread::sleep(StdDuration::from_millis(150));
            holder.execute_batch("COMMIT").unwrap();
        });

        let store = Store::open(&db.path).unwrap();
        release.join().unwrap();

        insert(&store, &english("Haus", "2026-10-31T12:00:00Z"));
        let raw = db.raw();
        assert_eq!(journal_mode(&raw), "wal");
        assert_eq!(user_version(&raw), 3);
    }

    #[test]
    fn apps_starting_together_on_a_fresh_file_all_open_it() {
        // Each round races several openers on a new file. They all see
        // version 0 at first, so only the re-check inside each migration's
        // transaction keeps the later ones from re-running a step ("table
        // captures already exists"). That depends on timing, hence rounds.
        const OPENERS: usize = 8;
        const ROUNDS: usize = 10;
        for round in 0..ROUNDS {
            let db = TempDb::new();
            let start = Arc::new(Barrier::new(OPENERS));
            let openers: Vec<_> = (0..OPENERS)
                .map(|_| {
                    let path = db.path.clone();
                    let start = Arc::clone(&start);
                    thread::spawn(move || {
                        start.wait();
                        Store::open(&path).map(drop)
                    })
                })
                .collect();
            for opener in openers {
                if let Err(err) = opener.join().unwrap() {
                    panic!("round {round}: {err}");
                }
            }
            let raw = db.raw();
            assert_eq!(user_version(&raw), 3, "round {round}");
            assert_eq!(journal_mode(&raw), "wal", "round {round}");
            for (table, columns) in EXPECTED_COLUMNS {
                assert_eq!(column_names(&raw, table), columns, "round {round}: {table}");
            }
        }
    }
}

mod captures {
    use super::*;

    fn full_english_card() -> Card {
        let mut card = Card::new(
            "serendipity",
            "glücklicher Zufall",
            "en",
            "de",
            at("2026-10-31T12:34:56.789Z"),
        );
        card.example_source = "It was pure serendipity.".into();
        card.example_target = "Es war reiner Zufall.".into();
        card.notes = "Rare word. More common synonym: luck".into();
        card.tags = vec![
            "travel".into(),
            "two words".into(),
            "quote \" and comma ,".into(),
        ];
        // NUL bytes would truncate a blob mistaken for a C string.
        card.audio_front = Some(vec![0xff, 0xfb, 0x00, 0x90, 0x00, 0x01]);
        card
    }

    fn full_chinese_card() -> Card {
        let mut card = Card::new(
            "學習",
            "lernen",
            "zh-CN",
            "de",
            at("2026-10-31T12:00:00.001Z"),
        );
        card.example_source = "我每天學習中文。".into();
        card.example_target = "Ich lerne jeden Tag Chinesisch.".into();
        card.tags = vec!["hsk-1".into()];
        card.audio_front = Some(vec![1, 2, 3]);
        card.script_metadata = ScriptMetadata::Chinese {
            pinyin: "xué xí".into(),
            hsk_level: Some(1),
            script_variant: ScriptVariant::Traditional,
        };
        card
    }

    #[test]
    fn cards_round_trip_with_and_without_script_metadata() {
        let store = Store::open_in_memory().unwrap();
        let mut unlevelled = chinese("您们", "nín men", None, "2026-10-31T12:00:00.002Z");
        unlevelled.audio_front = Some(Vec::new());
        let cases = [
            (full_english_card(), CaptureOrigin::Capture, false),
            (full_chinese_card(), CaptureOrigin::Curriculum, true),
            (unlevelled, CaptureOrigin::AnkiImport, false),
        ];
        for (card, origin, synced_to_anki) in cases {
            let id = store.insert_capture(&card, origin, synced_to_anki).unwrap();
            assert_eq!(
                store.get_capture(id).unwrap(),
                Some(StoredCapture {
                    id,
                    card,
                    origin,
                    synced_to_anki,
                })
            );
        }
        assert_eq!(store.get_capture(999).unwrap(), None);
    }

    #[test]
    fn card_fields_are_stored_in_the_documented_column_formats() {
        let db = TempDb::new();
        let store = db.open();
        let chinese_id = store
            .insert_capture(&full_chinese_card(), CaptureOrigin::Curriculum, true)
            .unwrap();
        let english_id = insert(&store, &english("Haus", "2026-10-31T12:00:00Z"));

        type Columns = (
            String,
            String,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
            String,
            i64,
            Option<Vec<u8>>,
        );
        let columns = |id: i64| -> Columns {
            db.raw()
                .query_row(
                    "SELECT captured_at, tags, script_kind, pinyin, hsk_level, script_variant, \
                     origin, synced_to_anki, audio_front FROM captures WHERE id = ?1",
                    [id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                            row.get(8)?,
                        ))
                    },
                )
                .unwrap()
        };
        assert_eq!(
            columns(chinese_id),
            (
                "2026-10-31T12:00:00.001Z".into(),
                r#"["hsk-1"]"#.into(),
                "chinese".into(),
                Some("xué xí".into()),
                Some(1),
                Some("traditional".into()),
                "curriculum".into(),
                1,
                Some(vec![1, 2, 3]),
            )
        );
        assert_eq!(
            columns(english_id),
            (
                "2026-10-31T12:00:00.000Z".into(),
                "[]".into(),
                "none".into(),
                None,
                None,
                None,
                "capture".into(),
                0,
                None,
            )
        );
    }

    #[test]
    fn recent_captures_are_newest_first_with_later_ids_first_on_ties() {
        let store = Store::open_in_memory().unwrap();
        // Inserted out of order; `a` and `c` share a timestamp, and `e` is a
        // whole second that a format without fractions would misplace.
        let a = insert(&store, &english("a", "2026-10-31T12:00:00.100Z"));
        let b = insert(&store, &english("b", "2026-10-31T12:00:00.900Z"));
        let c = insert(&store, &english("c", "2026-10-31T12:00:00.100Z"));
        let d = insert(&store, &english("d", "2026-10-31T11:59:59.999Z"));
        let e = insert(&store, &english("e", "2026-10-31T12:00:00.000Z"));

        assert_eq!(
            ids(&store.list_recent_captures(10).unwrap()),
            [b, c, a, e, d]
        );
        assert_eq!(ids(&store.list_recent_captures(2).unwrap()), [b, c]);
        assert!(store.list_recent_captures(0).unwrap().is_empty());
    }

    #[test]
    fn list_captures_is_oldest_first_and_filters_by_origin() {
        let store = Store::open_in_memory().unwrap();
        let late = insert(&store, &english("late", "2026-10-31T12:00:00.900Z"));
        let tie_a = insert(&store, &english("tie-a", "2026-10-31T12:00:00.100Z"));
        let curriculum = store
            .insert_capture(
                &chinese("学习", "xué xí", Some(1), "2026-10-31T11:00:00.000Z"),
                CaptureOrigin::Curriculum,
                false,
            )
            .unwrap();
        let tie_b = insert(&store, &english("tie-b", "2026-10-31T12:00:00.100Z"));

        assert_eq!(
            ids(&store.list_captures(None).unwrap()),
            [curriculum, tie_a, tie_b, late]
        );
        assert_eq!(
            ids(&store.list_captures(Some(CaptureOrigin::Capture)).unwrap()),
            [tie_a, tie_b, late]
        );
        assert_eq!(
            ids(&store
                .list_captures(Some(CaptureOrigin::Curriculum))
                .unwrap()),
            [curriculum]
        );
        assert!(store
            .list_captures(Some(CaptureOrigin::AnkiImport))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn unsynced_captures_are_listed_by_id_until_marked_synced() {
        let store = Store::open_in_memory().unwrap();
        let first = insert(&store, &english("first", "2026-10-31T12:00:00.000Z"));
        let synced = store
            .insert_capture(
                &english("synced", "2026-10-31T12:01:00.000Z"),
                CaptureOrigin::Capture,
                true,
            )
            .unwrap();
        // Captured before `first` but inserted after it: the order is by id.
        let third = insert(&store, &english("third", "2026-10-30T12:00:00.000Z"));
        assert_eq!(
            ids(&store.list_unsynced_captures().unwrap()),
            [first, third]
        );

        store.set_synced_to_anki(first, true).unwrap();
        assert_eq!(ids(&store.list_unsynced_captures().unwrap()), [third]);
        assert!(store.get_capture(first).unwrap().unwrap().synced_to_anki);

        store.set_synced_to_anki(synced, false).unwrap();
        assert_eq!(
            ids(&store.list_unsynced_captures().unwrap()),
            [synced, third]
        );

        // Setting the value it already has is not an error.
        store.set_synced_to_anki(first, true).unwrap();
        assert!(matches!(
            store.set_synced_to_anki(999, true),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn deleting_a_capture_cascades_to_its_srs_state_and_review_log() {
        let store = Store::open_in_memory().unwrap();
        let doomed = insert(
            &store,
            &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:00Z"),
        );
        let kept = insert(
            &store,
            &chinese("你好", "nǐ hǎo", Some(1), "2026-10-31T12:00:00Z"),
        );
        for id in [doomed, kept] {
            store
                .upsert_srs_state(&new_state(id, "2026-11-01T00:00:00.000Z"))
                .unwrap();
            store
                .insert_review_log(&review(id, 3, "2026-10-31T09:00:00.000Z"))
                .unwrap();
            store
                .insert_review_log(&review(id, 1, "2026-10-31T09:05:00.000Z"))
                .unwrap();
        }

        assert!(store.delete_capture(doomed).unwrap());

        assert_eq!(store.get_capture(doomed).unwrap(), None);
        assert_eq!(store.get_srs_state(doomed).unwrap(), None);
        assert!(store.review_log_for_card(doomed).unwrap().is_empty());
        assert_eq!(store.count_srs_states().unwrap(), 1);
        assert!(store.get_srs_state(kept).unwrap().is_some());
        assert_eq!(store.review_log_for_card(kept).unwrap().len(), 2);

        assert!(!store.delete_capture(doomed).unwrap());
        assert!(!store.delete_capture(4242).unwrap());
    }

    #[test]
    fn captures_are_found_by_exact_front_within_a_script_kind() {
        let store = Store::open_in_memory().unwrap();
        let study = insert(
            &store,
            &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:00Z"),
        );
        let plain = insert(&store, &english("学习", "2026-10-31T12:00:01Z"));
        insert(
            &store,
            &chinese("你好", "nǐ hǎo", Some(1), "2026-10-31T12:00:02Z"),
        );
        insert(
            &store,
            &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:03Z"),
        );
        insert(&store, &english("Haus", "2026-10-31T12:00:04Z"));

        let found = |front: &str, kind: &str| {
            store
                .find_capture_by_front(front, kind)
                .unwrap()
                .map(|capture| capture.id)
        };
        assert_eq!(found("学习", "chinese"), Some(study), "the oldest match");
        assert_eq!(found("学习", "none"), Some(plain));
        assert_eq!(found("Haus", "chinese"), None);
        assert_eq!(found(" 学习", "chinese"), None, "fronts match exactly");

        assert_eq!(
            store.chinese_capture_fronts().unwrap(),
            HashSet::from(["学习".to_string(), "你好".to_string()])
        );
    }

    #[test]
    fn updating_a_card_rewrites_its_fields_but_keeps_origin_and_sync_state() {
        let db = TempDb::new();
        let store = db.open();
        let id = store
            .insert_capture(&full_chinese_card(), CaptureOrigin::Curriculum, true)
            .unwrap();

        let mut edited = full_english_card();
        edited.audio_front = None;
        store.update_capture_card(id, &edited).unwrap();
        assert_eq!(
            store.get_capture(id).unwrap(),
            Some(StoredCapture {
                id,
                card: edited.clone(),
                origin: CaptureOrigin::Curriculum,
                synced_to_anki: true,
            })
        );
        // Dropping the Chinese metadata clears its columns instead of leaving
        // stale values behind.
        let chinese_columns: (Option<String>, Option<i64>, Option<String>, Option<Vec<u8>>) = db
            .raw()
            .query_row(
                "SELECT pinyin, hsk_level, script_variant, audio_front FROM captures \
                 WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(chinese_columns, (None, None, None, None));

        store.update_capture_card(id, &full_chinese_card()).unwrap();
        assert_eq!(
            store.get_capture(id).unwrap().unwrap().card,
            full_chinese_card()
        );

        assert!(matches!(
            store.update_capture_card(999, &edited),
            Err(StoreError::NotFound(_))
        ));
        assert_eq!(ids(&store.list_recent_captures(10).unwrap()), [id]);
    }

    #[test]
    fn a_card_dated_past_year_9999_is_refused_and_the_history_stays_readable() {
        // What JavaScript's `toISOString()` gives for year 10000, and serde
        // accepts it. Written as is, the row would not parse back, and every
        // listing that reached it would fail.
        let far_future: DateTime<Utc> =
            serde_json::from_str("\"+010000-01-01T00:00:00.000Z\"").unwrap();
        let store = Store::open_in_memory().unwrap();
        let kept = insert(&store, &english("Haus", "2026-10-31T12:00:00Z"));
        let card = english("Zukunft", "2026-10-31T12:00:00Z");
        let card = Card {
            captured_at: far_future,
            ..card
        };

        assert!(matches!(
            store.insert_capture(&card, CaptureOrigin::Capture, false),
            Err(StoreError::Invalid(message)) if message.contains("0000 to 9999")
        ));
        assert!(matches!(
            store.update_capture_card(kept, &card),
            Err(StoreError::Invalid(_))
        ));

        assert_eq!(ids(&store.list_recent_captures(10).unwrap()), [kept]);
        assert_eq!(fronts(&store), ["Haus"]);
    }

    #[test]
    fn timestamps_written_by_other_tools_are_read_leniently() {
        let db = TempDb::new();
        let store = db.open();
        let raw = db.raw();
        let written = [
            (1, "2026-10-31T12:00:00Z"),
            (2, "2026-10-31T13:00:00.5+01:00"),
            (3, "2026-10-31 12:00:00"),
        ];
        for (id, text) in written {
            raw.execute(
                "INSERT INTO captures (id, front, back, source_lang, target_lang, captured_at) \
                 VALUES (?1, 'w', 'w', 'en', 'de', ?2)",
                (id, text),
            )
            .unwrap();
        }
        let captured_at = |id: i64| store.get_capture(id).unwrap().unwrap().card.captured_at;
        assert_eq!(captured_at(1), at("2026-10-31T12:00:00Z"));
        assert_eq!(captured_at(2), at("2026-10-31T12:00:00.500Z"));
        assert_eq!(captured_at(3), at("2026-10-31T12:00:00Z"));
    }

    #[test]
    fn values_this_build_cannot_interpret_are_errors_not_guesses() {
        let db = TempDb::new();
        let store = db.open();
        let id = insert(
            &store,
            &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:00Z"),
        );
        let raw = db.raw();
        let corruptions = [
            ("script_kind", "japanese", false),
            ("script_variant", "cursive", false),
            ("origin", "martian", false),
            ("captured_at", "yesterday", false),
            ("tags", "travel, b1", true),
        ];
        for (column, value, is_json) in corruptions {
            let original: String = raw
                .query_row(
                    &format!("SELECT {column} FROM captures WHERE id = ?1"),
                    [id],
                    |row| row.get(0),
                )
                .unwrap();
            let set = format!("UPDATE captures SET {column} = ?1 WHERE id = ?2");
            raw.execute(&set, (value, id)).unwrap();

            match store.get_capture(id) {
                Err(StoreError::Json(_)) if is_json => {}
                Err(StoreError::Invalid(message)) if !is_json => assert!(
                    message.starts_with(&format!("captures row {id}: ")) && message.contains(value),
                    "{message}"
                ),
                other => panic!("{column} = {value:?} gave {other:?}"),
            }
            assert!(store.list_recent_captures(10).is_err());

            raw.execute(&set, (original, id)).unwrap();
            assert!(store.get_capture(id).unwrap().is_some());
        }
    }
}

mod curriculum {
    use super::*;

    /// Three items given out of `curriculum_order`, with order 10 to catch a
    /// text sort; `学习` has every optional field set.
    fn sample() -> Vec<CurriculumItem> {
        let study = CurriculumItem {
            traditional: Some("學習".into()),
            hsk_level: 1,
            frequency: Some(512),
            pinyin: "xué xí".into(),
            pos: Some("v".into()),
            meaning_en: Some("to study".into()),
            meaning_de: Some("lernen".into()),
            cloze_sentence: Some("我每天__中文。".into()),
            example_sentence_source: Some("我每天学习中文。".into()),
            example_sentence_pinyin: Some("wǒ měi tiān xué xí zhōng wén".into()),
            example_sentence_target: Some("Ich lerne jeden Tag Chinesisch.".into()),
            vocab_audio_path: Some("audio/xuexi.mp3".into()),
            sentence_audio_path: Some("audio/xuexi_sentence.mp3".into()),
            image_path: Some("img/xuexi.png".into()),
            ..curriculum_item("学习", 2)
        };
        vec![
            study,
            curriculum_item("中文", 10),
            curriculum_item("你好", 1),
        ]
    }

    fn in_order(mut items: Vec<CurriculumItem>) -> Vec<CurriculumItem> {
        items.sort_by_key(|item| item.curriculum_order);
        items
    }

    fn stored_fingerprint(store: &Store) -> Option<String> {
        store.get_state(CURRICULUM_FINGERPRINT_KEY).unwrap()
    }

    #[test]
    fn replace_curriculum_rewrites_the_whole_table() {
        let mut store = Store::open_in_memory().unwrap();
        assert_eq!(store.curriculum_count().unwrap(), 0);
        let items = sample();

        store.replace_curriculum(&items).unwrap();

        assert_eq!(store.curriculum_count().unwrap(), 3);
        assert_eq!(
            store.get_curriculum_item("学习").unwrap().as_ref(),
            Some(&items[0])
        );
        assert_eq!(
            store.get_curriculum_item("中文").unwrap().as_ref(),
            Some(&items[1])
        );
        assert_eq!(store.get_curriculum_item("没有").unwrap(), None);
        let hanzi: Vec<String> = store
            .curriculum_in_order()
            .unwrap()
            .into_iter()
            .map(|item| item.hanzi)
            .collect();
        assert_eq!(hanzi, ["你好", "学习", "中文"]);
        assert_eq!(
            stored_fingerprint(&store),
            Some(curriculum_fingerprint(&items))
        );

        let replacement = vec![curriculum_item("再见", 1)];
        store.replace_curriculum(&replacement).unwrap();
        assert_eq!(store.curriculum_in_order().unwrap(), replacement);
        assert_eq!(
            stored_fingerprint(&store),
            Some(curriculum_fingerprint(&replacement))
        );
    }

    #[test]
    fn seeding_writes_once_and_again_only_when_the_curriculum_changes() {
        let db = TempDb::new();
        let mut store = db.open();
        let items = sample();

        assert!(store.ensure_curriculum_seeded(&items).unwrap());
        assert_eq!(
            store.curriculum_in_order().unwrap(),
            in_order(items.clone())
        );
        assert_eq!(
            stored_fingerprint(&store),
            Some(curriculum_fingerprint(&items))
        );

        // The same items in another order are the same curriculum. A hand
        // edit survives the call, so the table really was not rewritten.
        db.raw()
            .execute(
                "UPDATE hsk_curriculum SET meaning_de = 'hand edit' WHERE hanzi = '学习'",
                [],
            )
            .unwrap();
        let reversed: Vec<CurriculumItem> = items.iter().rev().cloned().collect();
        assert!(!store.ensure_curriculum_seeded(&reversed).unwrap());
        let edited = store.get_curriculum_item("学习").unwrap().unwrap();
        assert_eq!(edited.meaning_de.as_deref(), Some("hand edit"));

        // The fingerprint lives in the file: a restart, or the other app,
        // does not reseed either.
        drop(store);
        let mut store = db.open();
        assert!(!store.ensure_curriculum_seeded(&items).unwrap());

        let mut changed = items.clone();
        changed[0].meaning_de = Some("studieren".into());
        assert!(store.ensure_curriculum_seeded(&changed).unwrap());
        assert_eq!(
            store.get_curriculum_item("学习").unwrap().as_ref(),
            Some(&changed[0])
        );
        assert!(!store.ensure_curriculum_seeded(&changed).unwrap());

        // Rows deleted behind the fingerprint's back are restored.
        db.raw()
            .execute("DELETE FROM hsk_curriculum WHERE hanzi = '你好'", [])
            .unwrap();
        assert!(store.ensure_curriculum_seeded(&changed).unwrap());
        assert_eq!(store.curriculum_in_order().unwrap(), in_order(changed));
    }

    #[test]
    fn a_rejected_curriculum_leaves_the_previous_one_in_place() {
        let mut store = Store::open_in_memory().unwrap();
        let items = sample();
        store.replace_curriculum(&items).unwrap();
        let fingerprint = stored_fingerprint(&store);

        // The first item is inserted before the second one fails, so only a
        // real rollback restores the old table.
        let same_order = vec![curriculum_item("再见", 5), curriculum_item("谢谢", 5)];
        assert!(matches!(
            store.replace_curriculum(&same_order),
            Err(StoreError::Sqlite(_))
        ));
        assert!(matches!(
            store.ensure_curriculum_seeded(&same_order),
            Err(StoreError::Sqlite(_))
        ));
        let same_hanzi = vec![curriculum_item("再见", 5), curriculum_item("再见", 6)];
        assert!(matches!(
            store.replace_curriculum(&same_hanzi),
            Err(StoreError::Sqlite(_))
        ));

        assert_eq!(store.curriculum_in_order().unwrap(), in_order(items));
        assert_eq!(stored_fingerprint(&store), fingerprint);
    }
}

mod srs {
    use super::*;

    #[test]
    fn srs_state_is_inserted_then_replaced_and_counted() {
        let store = Store::open_in_memory().unwrap();
        let first = insert(
            &store,
            &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:00Z"),
        );
        let second = insert(
            &store,
            &chinese("你好", "nǐ hǎo", Some(1), "2026-10-31T12:00:00Z"),
        );
        assert_eq!(store.get_srs_state(first).unwrap(), None);
        assert_eq!(store.count_srs_states().unwrap(), 0);

        let fresh = new_state(first, "2026-10-31T12:00:00.000Z");
        store.upsert_srs_state(&fresh).unwrap();
        assert_eq!(store.get_srs_state(first).unwrap().as_ref(), Some(&fresh));

        // Every field but the key differs from `fresh`, so a column the
        // upsert forgets to replace shows up.
        let reviewed = SrsStateRow {
            card_id: first,
            algorithm: "fsrs-6".into(),
            stability: Some(2.3065),
            difficulty: Some(5.114_6),
            interval_days: 3,
            repetitions: 1,
            lapses: 1,
            fsrs_state: "review".into(),
            elapsed_days: 2,
            due_date: at("2026-11-03T12:00:00.000Z"),
            last_reviewed_at: Some(at("2026-10-31T12:00:05.250Z")),
            last_rating: Some(3),
        };
        store.upsert_srs_state(&reviewed).unwrap();
        assert_eq!(
            store.get_srs_state(first).unwrap().as_ref(),
            Some(&reviewed)
        );
        assert_eq!(store.count_srs_states().unwrap(), 1);

        // Every column is replaced, including back to NULL.
        store.upsert_srs_state(&fresh).unwrap();
        assert_eq!(store.get_srs_state(first).unwrap().as_ref(), Some(&fresh));

        store
            .upsert_srs_state(&new_state(second, "2026-10-31T12:00:00.000Z"))
            .unwrap();
        assert_eq!(store.count_srs_states().unwrap(), 2);

        let out_of_range = SrsStateRow {
            last_rating: Some(5),
            ..fresh
        };
        assert!(matches!(
            store.upsert_srs_state(&out_of_range),
            Err(StoreError::Sqlite(_))
        ));
    }

    #[test]
    fn progress_rows_for_a_missing_card_are_rejected() {
        let store = Store::open_in_memory().unwrap();
        assert!(matches!(
            store.upsert_srs_state(&new_state(77, "2026-10-31T12:00:00.000Z")),
            Err(StoreError::Sqlite(_))
        ));
        assert!(matches!(
            store.insert_review_log(&review(77, 3, "2026-10-31T12:00:00.000Z")),
            Err(StoreError::Sqlite(_))
        ));
        assert_eq!(store.count_srs_states().unwrap(), 0);
    }

    #[test]
    fn due_states_are_strictly_before_the_bound_in_due_then_card_order() {
        let store = Store::open_in_memory().unwrap();
        let card: Vec<i64> = (0..7)
            .map(|n| {
                insert(
                    &store,
                    &english(&format!("word {n}"), "2026-10-31T00:00:00Z"),
                )
            })
            .collect();
        // Upserted in an order that matches neither the due order nor the
        // card ids. Several instants differ only in their milliseconds.
        let dues = [
            (card[3], "2026-10-31T12:00:00.500Z"),
            (card[0], "2026-10-31T12:00:01.000Z"),
            (card[5], "2026-10-31T12:00:00.050Z"),
            (card[1], "2026-10-31T12:00:00.500Z"),
            (card[6], "2026-10-31T12:00:00.000Z"),
            (card[2], "2026-10-31T11:59:59.999Z"),
            (card[4], "2026-11-01T00:00:00.000Z"),
        ];
        for (id, due) in dues {
            store.upsert_srs_state(&new_state(id, due)).unwrap();
        }
        let due_ids = |before: DateTime<Utc>| -> Vec<i64> {
            store
                .list_srs_due_before(before)
                .unwrap()
                .iter()
                .map(|state| state.card_id)
                .collect()
        };

        let one_second = at("2026-10-31T12:00:01.000Z");
        assert_eq!(
            due_ids(one_second),
            [card[2], card[6], card[5], card[1], card[3]]
        );
        assert_eq!(
            due_ids(one_second + Duration::milliseconds(1)),
            [card[2], card[6], card[5], card[1], card[3], card[0]]
        );
        let half = at("2026-10-31T12:00:00.500Z");
        assert_eq!(due_ids(half), [card[2], card[6], card[5]]);
        // A bound inside a millisecond is still after a state stored at it.
        assert_eq!(
            due_ids(half + Duration::microseconds(300)),
            [card[2], card[6], card[5], card[1], card[3]]
        );
        assert!(due_ids(at("2026-10-31T11:59:59.999Z")).is_empty());

        assert_eq!(
            store
                .list_srs_due_before(at("2026-10-31T12:00:00.001Z"))
                .unwrap(),
            [
                new_state(card[2], "2026-10-31T11:59:59.999Z"),
                new_state(card[6], "2026-10-31T12:00:00.000Z"),
            ]
        );
    }

    #[test]
    fn a_bound_past_year_9999_selects_every_state() {
        let store = Store::open_in_memory().unwrap();
        let soon = insert(&store, &english("soon", "2026-10-31T00:00:00Z"));
        let last = insert(&store, &english("last", "2026-10-31T00:00:00Z"));
        let last_state = new_state(last, "9999-12-31T23:59:59.999Z");
        let soon_state = new_state(soon, "2026-10-31T12:00:00.000Z");
        store.upsert_srs_state(&last_state).unwrap();
        store.upsert_srs_state(&soon_state).unwrap();
        let due_ids = |before: DateTime<Utc>| -> Vec<i64> {
            store
                .list_srs_due_before(before)
                .unwrap()
                .iter()
                .map(|state| state.card_id)
                .collect()
        };

        // `MAX_UTC` as "no upper bound": its text, `+262142-...`, would sort
        // below every stored instant and select nothing.
        assert_eq!(due_ids(DateTime::<Utc>::MAX_UTC), [soon, last]);
        assert_eq!(due_ids(at("9999-12-31T23:59:59.999Z")), [soon]);
        assert!(due_ids(DateTime::<Utc>::MIN_UTC).is_empty());

        // A state that would not read back is refused rather than stored.
        let unreadable = SrsStateRow {
            last_reviewed_at: Some(DateTime::<Utc>::MAX_UTC),
            ..soon_state.clone()
        };
        assert!(matches!(
            store.upsert_srs_state(&unreadable),
            Err(StoreError::Invalid(_))
        ));
        let unreadable = SrsStateRow {
            due_date: DateTime::<Utc>::MAX_UTC,
            ..soon_state.clone()
        };
        assert!(matches!(
            store.upsert_srs_state(&unreadable),
            Err(StoreError::Invalid(_))
        ));
        let far_review = ReviewLogRow {
            reviewed_at: DateTime::<Utc>::MAX_UTC,
            ..review(soon, 3, "2026-10-31T12:00:00.000Z")
        };
        assert!(matches!(
            store.insert_review_log(&far_review),
            Err(StoreError::Invalid(_))
        ));
        assert_eq!(store.get_srs_state(soon).unwrap(), Some(soon_state));
        assert!(store.review_log_for_card(soon).unwrap().is_empty());
    }

    #[test]
    fn malformed_scheduling_values_are_errors_that_name_their_row() {
        fn invalid_message<T: std::fmt::Debug>(result: StoreResult<T>) -> String {
            match result {
                Err(StoreError::Invalid(message)) => message,
                other => panic!("expected StoreError::Invalid, got {other:?}"),
            }
        }

        let db = TempDb::new();
        let store = db.open();
        let card = insert(&store, &english("Haus", "2026-10-31T12:00:00Z"));
        store
            .upsert_srs_state(&new_state(card, "2026-10-31T12:00:00.000Z"))
            .unwrap();
        let log = store
            .insert_review_log(&review(card, 3, "2026-10-31T12:00:00.000Z"))
            .unwrap();
        let raw = db.raw();
        raw.execute(
            "UPDATE srs_state SET last_reviewed_at = 'soon' WHERE card_id = ?1",
            [card],
        )
        .unwrap();
        raw.execute(
            "UPDATE review_log SET session_date = '31.10.2026' WHERE id = ?1",
            [log],
        )
        .unwrap();

        let message = invalid_message(store.get_srs_state(card));
        assert!(
            message.starts_with(&format!("srs_state row {card}: ")) && message.contains("soon"),
            "{message}"
        );
        assert_eq!(
            invalid_message(store.list_srs_due_before(at("2026-11-01T00:00:00Z"))),
            message
        );
        let message = invalid_message(store.review_log_for_card(card));
        assert!(
            message.starts_with(&format!("review_log row {log}: "))
                && message.contains("31.10.2026"),
            "{message}"
        );
    }

    #[test]
    fn review_log_rows_get_store_ids_and_come_back_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        let card = insert(
            &store,
            &chinese("学习", "xué xí", Some(1), "2026-10-31T08:00:00Z"),
        );
        let other = insert(
            &store,
            &chinese("你好", "nǐ hǎo", Some(1), "2026-10-31T08:00:00Z"),
        );

        let later = ReviewLogRow {
            id: 999,
            ..review(card, 3, "2026-10-31T09:05:00.000Z")
        };
        let earlier = ReviewLogRow {
            session_date: day("2026-10-30"),
            state_before: "new".into(),
            scheduled_days: 0,
            elapsed_days: 3,
            ..review(card, 1, "2026-10-31T09:00:00.000Z")
        };
        let elsewhere = review(other, 4, "2026-10-31T08:00:00.000Z");
        let later_id = store.insert_review_log(&later).unwrap();
        let elsewhere_id = store.insert_review_log(&elsewhere).unwrap();
        let earlier_id = store.insert_review_log(&earlier).unwrap();

        assert_ne!(later_id, 999, "row.id is ignored");
        assert_eq!(
            store.review_log_for_card(card).unwrap(),
            [
                ReviewLogRow {
                    id: earlier_id,
                    ..earlier
                },
                ReviewLogRow {
                    id: later_id,
                    ..later
                },
            ]
        );
        assert_eq!(
            store.review_log_for_card(other).unwrap(),
            [ReviewLogRow {
                id: elsewhere_id,
                ..elsewhere
            }]
        );
        assert!(store.review_log_for_card(12_345).unwrap().is_empty());

        for rating in [0, 5] {
            assert!(matches!(
                store.insert_review_log(&review(card, rating, "2026-10-31T10:00:00.000Z")),
                Err(StoreError::Sqlite(_))
            ));
        }
    }
}

mod progress {
    use super::*;

    #[test]
    fn daily_sessions_are_upserted_by_date() {
        let store = Store::open_in_memory().unwrap();
        let date = day("2026-10-31");
        assert_eq!(store.get_daily_session(date).unwrap(), None);

        let open = session("2026-10-31", None);
        store.upsert_daily_session(&open).unwrap();
        assert_eq!(store.get_daily_session(date).unwrap().as_ref(), Some(&open));

        let done = DailySessionRow {
            requested_new_count: 3,
            reviews_due_count: 13,
            reviews_completed: 13,
            new_items_completed: 3,
            completed_at: Some(at("2026-10-31T19:45:00.123Z")),
            ..open.clone()
        };
        store.upsert_daily_session(&done).unwrap();
        assert_eq!(store.get_daily_session(date).unwrap().as_ref(), Some(&done));
        assert_eq!(store.get_daily_session(day("2026-11-01")).unwrap(), None);

        // Upserting replaces every field, `completed_at` included.
        store.upsert_daily_session(&open).unwrap();
        assert_eq!(store.get_daily_session(date).unwrap().as_ref(), Some(&open));
        assert!(store.list_completed_session_dates().unwrap().is_empty());
    }

    #[test]
    fn completed_session_dates_are_listed_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        let sessions = [
            ("2026-11-03", Some("2026-11-03T20:00:00.000Z")),
            ("2026-10-30", Some("2026-10-30T20:00:00.000Z")),
            ("2026-11-02", None),
            ("2026-11-01", Some("2026-11-01T20:00:00.000Z")),
            ("2025-12-31", Some("2025-12-31T20:00:00.000Z")),
        ];
        for (date, completed_at) in sessions {
            store
                .upsert_daily_session(&session(date, completed_at))
                .unwrap();
        }
        assert_eq!(
            store.list_completed_session_dates().unwrap(),
            [
                day("2025-12-31"),
                day("2026-10-30"),
                day("2026-11-01"),
                day("2026-11-03"),
            ]
        );
    }

    #[test]
    fn app_state_values_are_set_replaced_and_deleted_per_key() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_state("last_prompted_date").unwrap(), None);

        store.set_state("last_prompted_date", "2026-10-30").unwrap();
        store.set_state("streak_freezes_available", "1").unwrap();
        store.set_state("last_prompted_date", "2026-10-31").unwrap();
        assert_eq!(
            store.get_state("last_prompted_date").unwrap().as_deref(),
            Some("2026-10-31")
        );

        store.delete_state("last_prompted_date").unwrap();
        assert_eq!(store.get_state("last_prompted_date").unwrap(), None);
        assert_eq!(
            store
                .get_state("streak_freezes_available")
                .unwrap()
                .as_deref(),
            Some("1")
        );
        store.delete_state("never_set").unwrap();

        store.set_state("empty", "").unwrap();
        assert_eq!(store.get_state("empty").unwrap().as_deref(), Some(""));
    }

    #[test]
    fn xp_is_summed_overall_and_per_local_date() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.xp_total().unwrap(), 0);
        assert_eq!(store.xp_for_date(day("2026-10-31")).unwrap(), 0);

        let first = store
            .insert_xp(
                day("2026-10-31"),
                10,
                "review",
                at("2026-10-31T09:00:00.000Z"),
            )
            .unwrap();
        let second = store
            .insert_xp(
                day("2026-10-31"),
                15,
                "new_item",
                at("2026-10-31T09:01:00.000Z"),
            )
            .unwrap();
        // Earned late on Oct 31 in UTC, but it was already Nov 1 locally.
        store
            .insert_xp(
                day("2026-11-01"),
                50,
                "session_complete",
                at("2026-10-31T23:30:00.000Z"),
            )
            .unwrap();
        store
            .insert_xp(
                day("2026-11-01"),
                5,
                "exercise",
                at("2026-11-01T08:00:00.000Z"),
            )
            .unwrap();

        assert!(second > first);
        assert_eq!(store.xp_total().unwrap(), 80);
        assert_eq!(store.xp_for_date(day("2026-10-31")).unwrap(), 25);
        assert_eq!(store.xp_for_date(day("2026-11-01")).unwrap(), 55);
        assert_eq!(store.xp_for_date(day("2026-11-02")).unwrap(), 0);
    }

    #[test]
    fn streak_freezes_are_idempotent_and_listed_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.list_streak_freeze_dates().unwrap().is_empty());
        for date in ["2026-11-02", "2026-10-28", "2026-11-02"] {
            store.insert_streak_freeze(day(date)).unwrap();
        }
        assert_eq!(
            store.list_streak_freeze_dates().unwrap(),
            [day("2026-10-28"), day("2026-11-02")]
        );
    }

    #[test]
    fn dates_past_year_9999_are_refused() {
        // Stored as `+10000-01-01`, they would sort before every other date.
        let store = Store::open_in_memory().unwrap();
        let far = NaiveDate::from_ymd_opt(10_000, 1, 1).unwrap();
        let refused = |result: StoreResult<()>| matches!(result, Err(StoreError::Invalid(_)));

        assert!(refused(store.insert_streak_freeze(far)));
        assert!(refused(
            store
                .insert_xp(far, 5, "exercise", at("2026-10-31T12:00:00Z"))
                .map(drop)
        ));
        assert!(refused(store.upsert_daily_session(&DailySessionRow {
            session_date: far,
            ..session("2026-10-31", None)
        })));
        assert!(refused(store.upsert_daily_session(&DailySessionRow {
            completed_at: Some(DateTime::<Utc>::MAX_UTC),
            ..session("2026-10-31", None)
        })));

        assert!(store.list_streak_freeze_dates().unwrap().is_empty());
        assert_eq!(store.xp_total().unwrap(), 0);
        assert_eq!(store.get_daily_session(day("2026-10-31")).unwrap(), None);
    }

    #[test]
    fn malformed_session_values_are_errors() {
        let db = TempDb::new();
        let store = db.open();
        let raw = db.raw();
        let insert_session = |date: &str, completed_at: &str| {
            raw.execute(
                "INSERT INTO daily_sessions (session_date, requested_new_count, \
                 reviews_due_count, completed_at) VALUES (?1, 1, 1, ?2)",
                [date, completed_at],
            )
            .unwrap();
        };

        insert_session("2026-10-31", "soon");
        let err = store.get_daily_session(day("2026-10-31")).unwrap_err();
        assert!(
            matches!(&err, StoreError::Invalid(message)
                if message.starts_with("daily_sessions row 2026-10-31: ") && message.contains("soon")),
            "{err:?}"
        );

        insert_session("31.10.2026", "2026-10-31T20:00:00.000Z");
        let err = store.list_completed_session_dates().unwrap_err();
        assert!(
            matches!(&err, StoreError::Invalid(message) if message.contains("31.10.2026")),
            "{err:?}"
        );
    }
}

mod transactions {
    use super::*;

    #[test]
    fn an_error_rolls_back_every_write_and_is_returned_unchanged() {
        let store = Store::open_in_memory().unwrap();
        let result: StoreResult<()> = store.with_transaction(|tx| {
            let id = tx.insert_capture(
                &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:00Z"),
                CaptureOrigin::Curriculum,
                false,
            )?;
            tx.upsert_srs_state(&new_state(id, "2026-10-31T12:00:00.000Z"))?;
            tx.insert_review_log(&review(id, 3, "2026-10-31T12:00:00.000Z"))?;
            tx.upsert_daily_session(&session("2026-10-31", None))?;
            tx.insert_xp(
                day("2026-10-31"),
                15,
                "new_item",
                at("2026-10-31T12:00:00Z"),
            )?;
            tx.insert_streak_freeze(day("2026-10-30"))?;
            tx.set_state("last_prompted_date", "2026-10-31")?;
            Err(StoreError::Invalid("abort".into()))
        });
        assert!(matches!(result, Err(StoreError::Invalid(message)) if message == "abort"));

        assert!(store.list_recent_captures(10).unwrap().is_empty());
        assert_eq!(store.count_srs_states().unwrap(), 0);
        assert_eq!(store.get_daily_session(day("2026-10-31")).unwrap(), None);
        assert_eq!(store.xp_total().unwrap(), 0);
        assert!(store.list_streak_freeze_dates().unwrap().is_empty());
        assert_eq!(store.get_state("last_prompted_date").unwrap(), None);

        // The connection is not left inside a transaction.
        let id = insert(&store, &english("Haus", "2026-10-31T12:00:00Z"));
        store
            .with_transaction(|tx| tx.set_synced_to_anki(id, true))
            .unwrap();
        assert!(store.get_capture(id).unwrap().unwrap().synced_to_anki);
    }

    #[test]
    fn ok_commits_and_returns_the_closures_value() {
        let db = TempDb::new();
        let store = db.open();
        let id = store
            .with_transaction(|tx| {
                let id = insert(
                    tx,
                    &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:00Z"),
                );
                tx.upsert_srs_state(&new_state(id, "2026-10-31T12:00:00.000Z"))?;
                Ok(id)
            })
            .unwrap();
        // Another connection sees it, so it was committed.
        let other = db.open();
        assert!(other.get_capture(id).unwrap().is_some());
        assert!(other.get_srs_state(id).unwrap().is_some());
    }

    #[test]
    fn nested_transactions_roll_back_only_the_failed_unit() {
        let store = Store::open_in_memory().unwrap();
        let inner_result = store
            .with_transaction(|tx| {
                insert(tx, &english("outer", "2026-10-31T12:00:00Z"));
                Ok(tx.with_transaction(|inner| {
                    insert(inner, &english("inner", "2026-10-31T12:00:01Z"));
                    Err::<(), _>(StoreError::Invalid("inner failed".into()))
                }))
            })
            .unwrap();
        assert!(matches!(inner_result, Err(StoreError::Invalid(_))));
        assert_eq!(fronts(&store), ["outer"]);

        // A successful inner unit still belongs to the outer transaction.
        let outer_result = store.with_transaction(|tx| {
            tx.with_transaction(|inner| {
                Ok(insert(inner, &english("nested", "2026-10-31T12:00:02Z")))
            })?;
            Err::<(), _>(StoreError::Invalid("outer failed".into()))
        });
        assert!(outer_result.is_err());
        assert_eq!(fronts(&store), ["outer"]);

        store
            .with_transaction(|tx| {
                tx.with_transaction(|inner| {
                    Ok(insert(inner, &english("nested", "2026-10-31T12:00:02Z")))
                })?;
                Ok(insert(tx, &english("after", "2026-10-31T12:00:03Z")))
            })
            .unwrap();
        assert_eq!(fronts(&store), ["after", "nested", "outer"]);
    }
}

/// Automator and Learn have the same file open at the same time.
mod two_apps {
    use super::*;

    /// How long one app holds the write lock while the other tries to write.
    /// Far below the five-second busy timeout, far above scheduling noise.
    const HOLD: StdDuration = StdDuration::from_millis(250);

    #[test]
    fn both_handles_read_and_write_the_same_rows() {
        let db = TempDb::new();
        let automator = db.open();
        let learn = db.open();

        let id = automator
            .insert_capture(
                &chinese("学习", "xué xí", Some(1), "2026-10-31T12:00:00Z"),
                CaptureOrigin::Capture,
                false,
            )
            .unwrap();
        assert_eq!(
            learn
                .get_capture(id)
                .unwrap()
                .map(|capture| capture.card.front),
            Some("学习".to_string())
        );

        learn
            .upsert_srs_state(&new_state(id, "2026-10-31T12:00:00.000Z"))
            .unwrap();
        learn
            .insert_review_log(&review(id, 3, "2026-10-31T12:05:00.000Z"))
            .unwrap();
        learn
            .insert_xp(day("2026-10-31"), 10, "review", at("2026-10-31T12:05:00Z"))
            .unwrap();
        assert!(automator.get_srs_state(id).unwrap().is_some());
        assert_eq!(automator.xp_total().unwrap(), 10);

        automator.set_synced_to_anki(id, true).unwrap();
        assert!(learn.get_capture(id).unwrap().unwrap().synced_to_anki);

        // Learn's connection enforces foreign keys as well.
        assert!(learn.delete_capture(id).unwrap());
        assert_eq!(automator.get_srs_state(id).unwrap(), None);
        assert!(automator.review_log_for_card(id).unwrap().is_empty());
    }

    #[test]
    fn a_write_waits_for_the_other_apps_transaction_instead_of_failing() {
        let db = TempDb::new();
        let automator = db.open();
        let learn = db.open();
        let (locked, wait_until_locked) = mpsc::channel();

        let worker = thread::spawn(move || {
            automator.with_transaction(|tx| {
                let id = insert(tx, &english("first", "2026-10-31T12:00:00Z"));
                locked.send(()).expect("the main thread is waiting");
                thread::sleep(HOLD);
                Ok(id)
            })
        });
        wait_until_locked.recv().unwrap();

        // With no busy timeout at all, this would fail at once with
        // SQLITE_BUSY instead of waiting for Automator's commit.
        let second = insert(&learn, &english("second", "2026-10-31T12:00:01Z"));
        let first = worker.join().unwrap().unwrap();
        assert_eq!(
            second,
            first + 1,
            "Learn's write landed after Automator committed"
        );
    }

    #[test]
    fn read_then_write_transactions_of_both_apps_take_turns() {
        let db = TempDb::new();
        let automator = db.open();
        let learn = db.open();
        let (started, wait_until_started) = mpsc::channel();

        let worker = thread::spawn(move || {
            automator.with_transaction(|tx| {
                let seen = tx.list_recent_captures(10)?.len();
                started.send(()).expect("the main thread is waiting");
                thread::sleep(HOLD);
                insert(tx, &english("from automator", "2026-10-31T12:00:00Z"));
                Ok(seen)
            })
        });
        wait_until_started.recv().unwrap();

        // Learn's transaction is attempted while Automator's is open. It only
        // begins once Automator commits, so it sees Automator's row, and
        // neither app's write is rejected for working from a stale snapshot.
        let learn_seen = learn
            .with_transaction(|tx| {
                let seen = tx.list_recent_captures(10)?.len();
                insert(tx, &english("from learn", "2026-10-31T12:00:01Z"));
                Ok(seen)
            })
            .unwrap();
        let automator_seen = worker.join().unwrap().unwrap();

        assert_eq!((automator_seen, learn_seen), (0, 1));
        assert_eq!(fronts(&learn), ["from learn", "from automator"]);
    }

    #[test]
    fn apps_seeding_at_the_same_time_write_the_curriculum_once() {
        let db = TempDb::new();
        drop(db.open());
        let items = vec![curriculum_item("你好", 1), curriculum_item("学习", 2)];

        // Both apps start seeding while a third connection holds the write
        // lock. Each "already seeded?" check has to happen inside the
        // seeding transaction: made before it, both checks would see an
        // empty table, and both apps would rewrite it.
        let holder = db.raw();
        holder.execute_batch("BEGIN IMMEDIATE").unwrap();
        let seeders: Vec<_> = (0..2)
            .map(|_| {
                let mut store = db.open();
                let items = items.clone();
                thread::spawn(move || store.ensure_curriculum_seeded(&items))
            })
            .collect();
        thread::sleep(HOLD);
        holder.execute_batch("COMMIT").unwrap();

        let wrote: Vec<bool> = seeders
            .into_iter()
            .map(|seeder| seeder.join().unwrap().unwrap())
            .collect();
        assert_eq!(wrote.iter().filter(|wrote| **wrote).count(), 1, "{wrote:?}");
        assert_eq!(db.open().curriculum_in_order().unwrap(), items);
    }
}
