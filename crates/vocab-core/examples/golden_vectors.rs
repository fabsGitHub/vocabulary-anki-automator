//! Golden vectors for the TypeScript port of vocab-core that Learn's web
//! build runs on (docs/architecture.md § 10, § 12 item 7).
//!
//! A browser cannot run the Rust core: compiling it to WASM needed a
//! toolchain change that was not approved, so `apps/learn/src/platform/web`
//! re-implements, module by module, the parts Learn uses. A port drifts
//! silently, so it is pinned to the real thing: this example feeds the Rust
//! implementations deterministic inputs and writes the inputs together with
//! the outputs to `apps/learn/src/platform/web/__fixtures__/golden.json`.
//! Vitest replays every input through the port and expects the same outputs.
//!
//! ```text
//! cargo run -p vocab-core --example golden_vectors             # rewrite the file
//! cargo run -p vocab-core --example golden_vectors -- --check  # exit 1 when it is stale
//! ```
//!
//! - **Deterministic.** Inputs are literals or drawn from `Mulberry32` with
//!   fixed seeds, every clock is a `FixedClock`, and sets are sorted before
//!   they are written, so every run writes the same file.
//! - **The shapes are the IPC shapes.** Results are serde's JSON of the Rust
//!   types, which is what the Tauri commands send, so the port's serializers
//!   (instants in chrono's format, dates, tagged enums) are pinned as well.
//! - **Scenarios replay whole call sequences** against an in-memory store:
//!   the store-backed functions (queue, introducing words, reviews, the daily
//!   prompt, streaks) are only equal if they read and write the same rows.
//!   The `adapter` scenarios do the same one level up, for Learn's commands
//!   composed from the core the way vocab-learn composes them ([`learn`]).
//! - **Prettier owns the layout.** The repository's lint gate formats JSON
//!   with Prettier, so [`layout`] writes the file the way Prettier lays it out,
//!   and `--check` compares parsed values, not bytes: neither a reformat nor
//!   the last bit of an FSRS float from another platform's `exp`/`powf` counts
//!   as a difference.

use std::{error::Error, fmt::Display, fs, path::PathBuf, process::ExitCode};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use rs_fsrs::{Card as FsrsCard, Parameters, Rating, ReviewLog, State, FSRS};
use serde::Serialize;
use serde_json::{json, Value};
use vocab_core::{
    curriculum::bundled_curriculum,
    gamification::{
        apply_streak_freezes, award_xp, compute_streak, on_session_completed, reason,
        stats_summary, StreakSummary, STREAK_FREEZES_KEY, XP_EXERCISE_CORRECT, XP_LESSON_COMPLETE,
        XP_NEW_ITEM, XP_REVIEW,
    },
    language::{cjk_chars, normalize_lang},
    lessons::{build_lessons, lessons_from_store},
    quiz::{
        check_answer, generate_exercise, Exercise, ExerciseAnswer, ExerciseKind,
        LearnedVocabularyPool, QuizError,
    },
    rng::Mulberry32,
    srs::{
        build_daily_queue, build_daily_queue_from, daily_prompt_state, dismiss_daily_prompt,
        fsrs_card_from_state, introduce_new_item, record_session_progress, reopen_daily_prompt,
        submit_daily_quota, submit_review, QueueItem, ReviewRating, SessionProgress, SrsScheduler,
        LAST_PROMPTED_KEY,
    },
    store::{curriculum_fingerprint, DailySessionRow, SrsStateRow, Store, StoreError, StoreResult},
    time::{self, format_date, local_day_bounds, Clock, FixedClock},
    CaptureOrigin, Card, CurriculumItem, ScriptMetadata, ScriptVariant, StoredCapture,
};

type BoxResult<T> = Result<T, Box<dyn Error>>;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("golden_vectors: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> BoxResult<ExitCode> {
    let check = std::env::args().skip(1).any(|arg| arg == "--check");
    let text = layout::to_json(&golden()?);
    let path = golden_path();
    if !check {
        fs::write(&path, text)?;
        println!("wrote {}", path.display());
        return Ok(ExitCode::SUCCESS);
    }
    let committed = fs::read_to_string(&path)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let committed: Value = serde_json::from_str(&committed)?;
    let regenerated: Value = serde_json::from_str(&text)?;
    match compare::first_difference(&committed, &regenerated) {
        None => {
            println!("{} is up to date", path.display());
            Ok(ExitCode::SUCCESS)
        }
        Some(at) => {
            eprintln!(
                "{} is stale (first difference at {at}); regenerate it with \
                 `cargo run -p vocab-core --example golden_vectors`",
                path.display()
            );
            Ok(ExitCode::FAILURE)
        }
    }
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/learn/src/platform/web/__fixtures__/golden.json")
}

fn golden() -> BoxResult<Value> {
    Ok(json!({
        "about": "Generated by crates/vocab-core/examples/golden_vectors.rs from the Rust core; \
                  regenerate, never edit by hand.",
        "mulberry32": mulberry32(),
        "language": language(),
        "time": time_vectors(),
        "fsrs": fsrs_reference(),
        "scheduler": scheduler_vectors(),
        "queue": queue_cases(),
        "streak": streak_cases(),
        "lessons": lesson_cases(),
        "fingerprint": fingerprint_cases()?,
        "exercises": exercise_vectors()?,
        "scenarios": scenarios(),
        "adapter": adapter_scenarios(),
    }))
}

// ---------------------------------------------------------------------------
// Fixture helpers

/// An instant from RFC 3339 text.
fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("fixture instants are RFC 3339")
        .with_timezone(&Utc)
}

fn offset(seconds: i32) -> FixedOffset {
    FixedOffset::east_opt(seconds).expect("fixture offsets are valid")
}

/// The instant of the wall-clock time `YYYY-MM-DD HH:MM:SS.fff` at `offset`.
fn local(offset: FixedOffset, text: &str) -> DateTime<Utc> {
    let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.3f")
        .expect("fixture local times parse");
    offset
        .from_local_datetime(&naive)
        .single()
        .expect("fixed offsets are unambiguous")
        .with_timezone(&Utc)
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("fixture dates parse")
}

fn to_json<T: Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).expect("golden values serialize")
}

/// `{"ok": value}` or `{"error": message}`, the message being what the Tauri
/// commands reject with.
fn outcome<T: Serialize, E: Display>(result: Result<T, E>) -> Value {
    match result {
        Ok(value) => json!({ "ok": to_json(&value) }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

fn chinese(front: &str, back: &str, target_lang: &str, pinyin: &str) -> Card {
    let mut card = Card::new(
        front,
        back,
        "zh-CN",
        target_lang,
        instant("2026-10-01T08:00:00Z"),
    );
    card.script_metadata = ScriptMetadata::Chinese {
        pinyin: pinyin.into(),
        hsk_level: Some(1),
        script_variant: ScriptVariant::Simplified,
    };
    card
}

fn plain(front: &str, back: &str, source_lang: &str, target_lang: &str) -> Card {
    Card::new(
        front,
        back,
        source_lang,
        target_lang,
        instant("2026-10-01T08:00:00Z"),
    )
}

fn with_example(mut card: Card, source: &str, target: &str) -> Card {
    card.example_source = source.into();
    card.example_target = target.into();
    card
}

fn with_audio(mut card: Card) -> Card {
    card.audio_front = Some(vec![0xff, 0xfb, 0x90, 0x00]);
    card
}

fn captured_at(mut card: Card, at: &str) -> Card {
    card.captured_at = instant(at);
    card
}

fn curriculum_item(hanzi: &str, order: i64) -> CurriculumItem {
    CurriculumItem {
        hanzi: hanzi.into(),
        traditional: None,
        hsk_level: 1,
        frequency: Some(order * 10),
        curriculum_order: order,
        pinyin: format!("pinyin of {hanzi}"),
        pos: None,
        meaning_en: Some(format!("{hanzi} (en)")),
        meaning_de: Some(format!("{hanzi} (de)")),
        cloze_sentence: None,
        example_sentence_source: None,
        example_sentence_pinyin: None,
        example_sentence_target: None,
        vocab_audio_path: None,
        sentence_audio_path: None,
        image_path: None,
    }
}

/// Curriculum rows in an order that is neither `curriculum_order` nor its
/// reverse, with gaps in the numbering.
fn scrambled_curriculum() -> Vec<CurriculumItem> {
    vec![
        curriculum_item("你好", 5),
        curriculum_item("我", 1),
        curriculum_item("学习", 30),
        curriculum_item("中文", 7),
        curriculum_item("是", 2),
        curriculum_item("好", 12),
    ]
}

/// A reviewed state due at `due`; the queue looks at nothing else.
fn due_state(card_id: i64, due: DateTime<Utc>) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: Some(4.0),
        difficulty: Some(5.0),
        interval_days: 4,
        repetitions: 2,
        lapses: 0,
        fsrs_state: "review".into(),
        elapsed_days: 4,
        due_date: due,
        last_reviewed_at: Some(due - Duration::days(4)),
        last_rating: Some(3),
    }
}

fn rating(number: u8) -> ReviewRating {
    ReviewRating::try_from(number).expect("fixture ratings are 1 to 4")
}

// ---------------------------------------------------------------------------
// Mulberry32 and language helpers

fn mulberry32() -> Value {
    let seeds = [
        0,
        1,
        7,
        42,
        1_000_000_007,
        0x8000_0000,
        0xDEAD_BEEF,
        u32::MAX,
    ];
    let outputs: Vec<Value> = seeds
        .iter()
        .map(|&seed| {
            let mut rng = Mulberry32::new(seed);
            let next_u32: Vec<u32> = (0..8).map(|_| rng.next_u32()).collect();
            let mut rng = Mulberry32::new(seed);
            let next_f64: Vec<f64> = (0..4).map(|_| rng.next_f64()).collect();
            let mut rng = Mulberry32::new(seed);
            let below: Vec<Value> = [0, 1, 2, 3, 7, 10, 1000, u32::MAX]
                .iter()
                .map(|&n| json!({ "n": n, "value": rng.below(n) }))
                .collect();
            json!({ "seed": seed, "next_u32": next_u32, "next_f64": next_f64, "below": below })
        })
        .collect();
    let shuffles: Vec<Value> = [
        (0, 0),
        (1, 1),
        (2, 2),
        (3, 5),
        (42, 10),
        (7, 33),
        (u32::MAX, 12),
    ]
    .iter()
    .map(|&(seed, len)| {
        let mut items: Vec<u32> = (0..len).collect();
        let mut rng = Mulberry32::new(seed);
        rng.shuffle(&mut items);
        // The next draw shows how many the shuffle consumed.
        json!({ "seed": seed, "len": len, "result": items, "next_u32": rng.next_u32() })
    })
    .collect();
    json!({ "outputs": outputs, "shuffles": shuffles })
}

fn language() -> Value {
    let codes = [
        "en",
        "EN",
        " En ",
        "DE",
        "fr",
        "ES",
        "It",
        "zh",
        "ZH",
        "zh-cn",
        "zh-CN",
        "ZH-CN",
        "zh_CN",
        "zh_cn",
        "zh-Hans",
        "zh_Hans",
        "zh-Hans-CN",
        "\tzh\n",
        "PT",
        " ja ",
        "en-US",
        "pt_BR",
        "zh-TW",
        "zh-Hant",
        "",
        "   ",
        // Rust trims Unicode White_Space: NEL and the ideographic space go,
        // the byte-order mark (not whitespace) stays.
        "\u{85}de\u{85}",
        "\u{3000}EN\u{3000}",
        "\u{feff}en",
        "\u{130}T",
        "\u{3a3}A",
        "A\u{3a3}",
    ];
    let normalize: Vec<Value> = codes
        .iter()
        .map(|code| json!({ "input": code, "output": normalize_lang(code) }))
        .collect();
    let texts = [
        "学习ABC学",
        "a\u{3400}b一",
        "no hanzi here",
        "。、！",
        "\u{4DBF}\u{4DC0}\u{9FFF}\u{A000}",
        "\u{1F600}学",
    ];
    let cjk: Vec<Value> = texts
        .iter()
        .map(|text| {
            let chars: Vec<String> = cjk_chars(text).iter().map(char::to_string).collect();
            json!({ "text": text, "chars": chars })
        })
        .collect();
    // `{:?}` of a string, which error messages quote values with.
    let debug: Vec<Value> = [
        "没有",
        " 学习",
        "",
        "a\"b\\c",
        "line\nbreak\ttab\rreturn\0nul",
        "\u{85}NEL",
        "\u{3000}ideographic space",
        "\u{a0}no-break space",
        "\u{feff}byte-order mark",
        "\u{200b}zero width",
        "e\u{301} decomposed",
        "\u{e9} composed",
        "it's",
        "\u{7f}\u{1b}",
        "\u{1F600} emoji",
        "\u{2028}line separator",
        "\u{e000}private use",
    ]
    .iter()
    .map(|text| json!({ "text": text, "debug": format!("{text:?}") }))
    .collect();
    json!({ "normalize_lang": normalize, "cjk_chars": cjk, "debug": debug })
}

// ---------------------------------------------------------------------------
// vocab_core::time

/// The local calendar: chrono's lenient `%Y-%m-%d` parsing (a stored prompt
/// date is parsed with it), day bounds and "today" in offsets on both sides
/// of UTC.
fn time_vectors() -> Value {
    let texts = [
        "2026-10-31",
        "2026-1-5",
        " 2026-01-05",
        "\u{3000}2026-1-5",
        "\u{85}2026-01-05",
        "\u{feff}2026-01-05",
        "2026- 1- 5",
        "2026 -01-05",
        "+2026-01-05",
        "-0001-12-31",
        "+10000-01-01",
        "026-01-01",
        "20261-01-01",
        "2026-01-005",
        "2026-01-05 ",
        "2026-02-29",
        "2028-02-29",
        "1900-02-29",
        "2000-02-29",
        "2026-13-01",
        "2026-00-10",
        "2026-04-31",
        "31.10.2026",
        "2026/10/31",
        "2026-01-0x",
        "",
    ];
    let parse_date: Vec<Value> = texts
        .iter()
        .map(|text| json!({ "text": text, "date": time::parse_date(text).map(format_date) }))
        .collect();
    let bounds: Vec<Value> = [
        ("2026-10-31", 3600),
        ("2026-11-01", -5 * 3600),
        ("2026-03-29", 19_800),
        ("1970-01-01", -34_200),
        ("2028-02-29", 9 * 3600),
        ("2026-12-31", 14 * 3600),
        ("2027-01-01", -12 * 3600),
    ]
    .iter()
    .map(|&(day, seconds)| {
        let (start, end) = local_day_bounds(date(day), offset(seconds));
        json!({ "date": day, "offset_seconds": seconds, "start": start, "end": end })
    })
    .collect();
    let today: Vec<Value> = [
        ("2026-10-31T23:30:00Z", 3600),
        ("2026-10-31T23:30:00Z", 0),
        ("2026-11-01T04:30:00Z", -5 * 3600),
        ("2026-11-01T04:59:59.999Z", -5 * 3600),
        ("2026-11-01T05:00:00Z", -5 * 3600),
        ("1969-12-31T23:59:59.999Z", 0),
        ("1970-01-01T00:00:00Z", -1),
        ("2026-12-31T10:00:00Z", 14 * 3600),
    ]
    .iter()
    .map(|&(now, seconds)| {
        let today = FixedClock::new(instant(now), offset(seconds)).today();
        json!({ "now": now, "offset_seconds": seconds, "today": today })
    })
    .collect();
    json!({ "parse_date": parse_date, "local_day_bounds": bounds, "today": today })
}

// ---------------------------------------------------------------------------
// rs-fsrs itself

/// rs-fsrs's `TEST_RATINGS`.
const TEST_RATINGS: [Rating; 13] = [
    Rating::Good,
    Rating::Good,
    Rating::Good,
    Rating::Good,
    Rating::Good,
    Rating::Good,
    Rating::Again,
    Rating::Again,
    Rating::Good,
    Rating::Good,
    Rating::Good,
    Rating::Good,
    Rating::Good,
];

/// rs-fsrs's custom `WEIGHTS`.
const WEIGHTS: [f64; 19] = [
    0.4197, 1.1869, 3.0412, 15.2441, 7.1434, 0.6477, 1.0007, 0.0674, 1.6597, 0.1712, 1.1178,
    2.0225, 0.0904, 0.3025, 2.1214, 0.2498, 2.9466, 0.4891, 0.6468,
];

/// Where every rs-fsrs reference case starts.
fn reference_start() -> DateTime<Utc> {
    instant("2022-11-29T12:30:00Z")
}

fn with_weights() -> Parameters {
    Parameters {
        w: WEIGHTS,
        ..Parameters::default()
    }
}

fn long_term(params: Parameters) -> Parameters {
    Parameters {
        enable_short_term: false,
        ..params
    }
}

fn params_json(params: &Parameters) -> Value {
    json!({
        "request_retention": params.request_retention,
        "maximum_interval": params.maximum_interval,
        "w": params.w.to_vec(),
        "enable_short_term": params.enable_short_term,
        "enable_fuzz": params.enable_fuzz,
    })
}

fn state_name(state: State) -> &'static str {
    match state {
        State::New => "new",
        State::Learning => "learning",
        State::Review => "review",
        State::Relearning => "relearning",
    }
}

fn fsrs_card_json(card: &FsrsCard) -> Value {
    json!({
        "due": card.due,
        "stability": card.stability,
        "difficulty": card.difficulty,
        "elapsed_days": card.elapsed_days,
        "scheduled_days": card.scheduled_days,
        "reps": card.reps,
        "lapses": card.lapses,
        "state": state_name(card.state),
        "last_review": card.last_review,
    })
}

fn fsrs_log_json(log: &ReviewLog) -> Value {
    json!({
        "rating": log.rating as u8,
        "elapsed_days": log.elapsed_days,
        "scheduled_days": log.scheduled_days,
        "state": state_name(log.state),
        "reviewed_date": log.reviewed_date,
    })
}

/// A card never reviewed, as rs-fsrs's tests start (without `Card::new()`,
/// which reads the system clock).
fn unseen(at: DateTime<Utc>) -> FsrsCard {
    FsrsCard {
        due: at,
        last_review: at,
        ..FsrsCard::default()
    }
}

/// One card through bare rs-fsrs. `next_step` gives the instant and rating
/// of review `index` of `card`, or `None` to stop; every step records all
/// four outcomes of `repeat`, and the given rating's card goes on.
fn bare_sequence(
    name: &str,
    params: Parameters,
    next_step: impl FnMut(usize, &FsrsCard, DateTime<Utc>) -> Option<(DateTime<Utc>, Rating)>,
) -> Value {
    bare_sequence_from(name, params, unseen(reference_start()), next_step)
}

/// [`bare_sequence`] from the card `start`. Every recorded memory value
/// must be finite: JSON cannot carry NaN, which serde would write as `null`.
fn bare_sequence_from(
    name: &str,
    params: Parameters,
    start: FsrsCard,
    mut next_step: impl FnMut(usize, &FsrsCard, DateTime<Utc>) -> Option<(DateTime<Utc>, Rating)>,
) -> Value {
    let fsrs = FSRS::new(params.clone());
    let mut card = start.clone();
    let mut now = reference_start();
    let mut steps = Vec::new();
    while let Some((at, rating)) = next_step(steps.len(), &card, now) {
        now = at;
        let record = fsrs.repeat(card.clone(), now);
        let outcomes: Vec<Value> = Rating::iter()
            .map(|rating| {
                let info = &record[rating];
                assert!(
                    info.card.stability.is_finite() && info.card.difficulty.is_finite(),
                    "{name}: step {} has a memory value JSON cannot carry",
                    steps.len()
                );
                json!({
                    "rating": *rating as u8,
                    "card": fsrs_card_json(&info.card),
                    "log": fsrs_log_json(&info.review_log),
                })
            })
            .collect();
        steps.push(json!({ "now": now, "rating": rating as u8, "outcomes": outcomes }));
        card = record[&rating].card.clone();
    }
    json!({
        "name": name,
        "parameters": params_json(&params),
        "start": fsrs_card_json(&start),
        "steps": steps,
    })
}

/// rs-fsrs's reference cases, which review each card at its due date.
fn at_due(
    ratings: &'static [Rating],
) -> impl FnMut(usize, &FsrsCard, DateTime<Utc>) -> Option<(DateTime<Utc>, Rating)> {
    move |index, card, _| {
        let at = if index == 0 {
            reference_start()
        } else {
            card.due
        };
        ratings.get(index).map(|rating| (at, *rating))
    }
}

/// Reviews at random instants after the previous one, forward only (bare
/// rs-fsrs has no guard against a clock set back; the wrapper does).
fn random_forward(
    seed: u32,
    len: usize,
) -> impl FnMut(usize, &FsrsCard, DateTime<Utc>) -> Option<(DateTime<Utc>, Rating)> {
    let mut rng = Mulberry32::new(seed);
    move |index, card, now| {
        if index == len {
            return None;
        }
        let rating = Rating::iter().nth(rng.below(4) as usize).copied()?;
        let at = match rng.below(5) {
            0 => now + Duration::minutes(1 + i64::from(rng.below(60))),
            1 => now + Duration::hours(1 + i64::from(rng.below(24))),
            2 => card.due.max(now),
            3 => card.due.max(now) + Duration::days(i64::from(rng.below(10))),
            _ => now + Duration::days(1 + i64::from(rng.below(100))),
        };
        Some((at, rating))
    }
}

/// `forward` Good reviews at the due date, then one review per entry of
/// `back`, each that long before the previous review. Bare rs-fsrs has no
/// guard against a clock set back, so its days since the last review turn
/// negative: `num_days` truncates them towards zero (half a day back is 0,
/// a day and a half is -1), retrievability exceeds 1 and recall stabilities
/// shrink, which reorders the intervals that `min`/`max` then clamp.
fn set_back(
    forward: usize,
    back: &'static [(Duration, Rating)],
) -> impl FnMut(usize, &FsrsCard, DateTime<Utc>) -> Option<(DateTime<Utc>, Rating)> {
    move |index, card, _| {
        if index < forward {
            let at = if index == 0 {
                reference_start()
            } else {
                card.due
            };
            return Some((at, Rating::Good));
        }
        let (by, rating) = back.get(index - forward)?;
        Some((card.last_review - *by, *rating))
    }
}

/// Half a day and then a day and a half before the last review.
const SET_BACK_BY_HOURS: [(Duration, Rating); 2] = [
    (Duration::hours(12), Rating::Good),
    (Duration::hours(36), Rating::Hard),
];

/// Seven hundred days before the last review of a well-learned card: its
/// recall stabilities collapse below the forget stability, so Again's
/// interval would exceed Hard's if `min` did not cap it.
const SET_BACK_BY_700_DAYS: [(Duration, Rating); 1] = [(Duration::days(700), Rating::Again)];

/// The first two reviews of a card, both Good.
const NEW_THEN_GOOD: [Rating; 2] = [Rating::Good, Rating::Good];

/// A card still `New` that carries days from an earlier life: the
/// long-term scheduler logs them as zero, the basic one as they are.
fn new_card_with_days() -> FsrsCard {
    FsrsCard {
        scheduled_days: 5,
        elapsed_days: 3,
        ..unseen(reference_start())
    }
}

fn fsrs_reference() -> Value {
    // rs-fsrs `test_basic_scheduler_memo_state`: fixed gaps, then a last Good.
    let memo_ratings = [
        Rating::Again,
        Rating::Good,
        Rating::Good,
        Rating::Good,
        Rating::Good,
        Rating::Good,
        Rating::Good,
    ];
    let gaps_in_days = [0, 0, 1, 3, 8, 21];
    let memo = bare_sequence(
        "basic_scheduler_memo_state",
        with_weights(),
        move |index, _, now| {
            let at = match index {
                0 => reference_start(),
                _ => now + Duration::days(*gaps_in_days.get(index - 1)?),
            };
            memo_ratings.get(index).map(|rating| (at, *rating))
        },
    );
    let sequences = vec![
        bare_sequence(
            "basic_scheduler_interval",
            Parameters::default(),
            at_due(&TEST_RATINGS),
        ),
        bare_sequence(
            "basic_scheduler_state",
            with_weights(),
            at_due(&TEST_RATINGS),
        ),
        memo,
        bare_sequence(
            "long_term_scheduler",
            long_term(with_weights()),
            at_due(&TEST_RATINGS),
        ),
        bare_sequence(
            "random_basic",
            Parameters::default(),
            random_forward(11, 16),
        ),
        bare_sequence(
            "random_long_term",
            long_term(Parameters::default()),
            random_forward(12, 16),
        ),
        bare_sequence(
            "random_weights_long_term",
            long_term(with_weights()),
            random_forward(13, 12),
        ),
        bare_sequence(
            "clock_set_back_basic",
            Parameters::default(),
            set_back(4, &SET_BACK_BY_HOURS),
        ),
        bare_sequence(
            "clock_set_back_long_term",
            long_term(Parameters::default()),
            set_back(5, &SET_BACK_BY_700_DAYS),
        ),
        bare_sequence_from(
            "new_card_with_days_basic",
            Parameters::default(),
            new_card_with_days(),
            at_due(&NEW_THEN_GOOD),
        ),
        bare_sequence_from(
            "new_card_with_days_long_term",
            long_term(Parameters::default()),
            new_card_with_days(),
            at_due(&NEW_THEN_GOOD),
        ),
    ];

    // rs-fsrs `test_get_retrievability`, plus the Easy card recalled late,
    // on time, early and before its last review.
    let fsrs = FSRS::default();
    let start = reference_start();
    let record = fsrs.repeat(unseen(start), start);
    let mut retrievability = Vec::new();
    for rating in Rating::iter() {
        let card = &record[rating].card;
        let mut instants = vec![card.due];
        if *rating == Rating::Easy {
            // Before the last review, `num_days` truncates towards zero:
            // half a day back is 0 days, a day and a half is -1.
            instants.extend([
                card.due - Duration::days(3),
                card.due + Duration::days(1),
                card.due + Duration::days(100),
                card.last_review - Duration::days(2),
                card.last_review - Duration::hours(12),
                card.last_review - Duration::hours(36),
            ]);
        }
        for at in instants {
            retrievability.push(json!({
                "card": fsrs_card_json(card),
                "at": at,
                "retrievability": card.get_retrievability(at),
            }));
        }
    }
    json!({ "sequences": sequences, "retrievability": retrievability })
}

// ---------------------------------------------------------------------------
// vocab_core::srs::SrsScheduler

/// Where the next review of a wrapper sequence happens: every kind of gap,
/// including a clock set back before the last review and instants that are
/// not whole milliseconds.
fn next_review_instant(
    rng: &mut Mulberry32,
    now: DateTime<Utc>,
    state: &SrsStateRow,
) -> DateTime<Utc> {
    let mut draw = |n: u32| i64::from(rng.below(n));
    match draw(10) {
        0 => now,
        1 => now + Duration::minutes(1 + draw(59)) + Duration::nanoseconds(draw(1_000_000)),
        2 => now + Duration::hours(1 + draw(23)),
        3 => state.due_date,
        4 => state.due_date + Duration::days(1 + draw(30)),
        5 => now + Duration::days(1 + draw(400)),
        6 => state.last_reviewed_at.unwrap_or(now) - Duration::hours(1 + draw(96)),
        7 => state.due_date - Duration::minutes(1 + draw(30)),
        8 => now + Duration::nanoseconds(1 + draw(999_999)),
        _ => state.due_date - Duration::milliseconds(1),
    }
}

fn wrapper_sequence(name: &str, params: Parameters, seed: u32, offset_seconds: i32) -> Value {
    let scheduler = SrsScheduler::new(params.clone());
    let mut rng = Mulberry32::new(seed);
    let start = instant("2026-10-31T08:00:00.123456789Z");
    let initial = scheduler.initial_state(7, start);
    let mut state = initial.clone();
    let mut now = start;
    let mut steps = Vec::new();
    for _ in 0..16 {
        let given = rating(1 + rng.below(4) as u8);
        now = next_review_instant(&mut rng, now, &state);
        let session_date = FixedClock::new(now, offset(offset_seconds)).today();
        let reviewed = scheduler.review(&state, given, now, session_date);
        steps.push(json!({
            "now": now,
            "rating": given,
            "session_date": session_date,
            "outcome": reviewed,
            "previews": scheduler.preview(&state, now),
        }));
        state = reviewed.next;
    }
    json!({
        "name": name,
        "parameters": params_json(&params),
        "initial": initial,
        "steps": steps,
    })
}

/// A reviewed card's state, valid for FSRS, for cases that edit one field.
fn review_state(card_id: i64) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: Some(12.5),
        difficulty: Some(6.25),
        interval_days: 9,
        repetitions: 4,
        lapses: 1,
        fsrs_state: "review".into(),
        elapsed_days: 3,
        due_date: instant("2026-11-09T08:00:00.000Z"),
        last_reviewed_at: Some(instant("2026-10-31T08:00:00.000Z")),
        last_rating: Some(3),
    }
}

/// Rows FSRS cannot interpret. NaN and infinite memory values are left out:
/// JSON cannot carry them (serde writes `null`, which is the "no stability"
/// case already here).
fn invalid_rows() -> Vec<(&'static str, SrsStateRow)> {
    let valid = review_state(7);
    let new = SrsScheduler::default().initial_state(7, instant("2026-10-31T08:00:00.000Z"));
    let max = i64::from(i32::MAX);
    vec![
        (
            "another algorithm",
            SrsStateRow {
                algorithm: "sm2".into(),
                ..valid.clone()
            },
        ),
        (
            "an unknown state",
            SrsStateRow {
                fsrs_state: "suspended".into(),
                ..valid.clone()
            },
        ),
        (
            "a capitalised state",
            SrsStateRow {
                fsrs_state: "Review".into(),
                ..valid.clone()
            },
        ),
        (
            "no stability",
            SrsStateRow {
                stability: None,
                ..valid.clone()
            },
        ),
        (
            "no difficulty",
            SrsStateRow {
                difficulty: None,
                ..valid.clone()
            },
        ),
        (
            "no last review",
            SrsStateRow {
                last_reviewed_at: None,
                ..valid.clone()
            },
        ),
        (
            "a zero stability",
            SrsStateRow {
                stability: Some(0.0),
                ..valid.clone()
            },
        ),
        (
            "a negative difficulty",
            SrsStateRow {
                difficulty: Some(-1.0),
                ..valid.clone()
            },
        ),
        (
            "a new card with a stability",
            SrsStateRow {
                stability: Some(3.0),
                ..new.clone()
            },
        ),
        (
            "a new card with a difficulty",
            SrsStateRow {
                difficulty: Some(5.0),
                ..new.clone()
            },
        ),
        (
            "a new card with a last review",
            SrsStateRow {
                last_reviewed_at: Some(instant("2026-11-01T00:00:00Z")),
                ..new
            },
        ),
        (
            "negative lapses",
            SrsStateRow {
                lapses: -1,
                ..valid.clone()
            },
        ),
        (
            "negative repetitions",
            SrsStateRow {
                repetitions: -2,
                ..valid.clone()
            },
        ),
        (
            "repetitions past i32",
            SrsStateRow {
                repetitions: max + 1,
                ..valid.clone()
            },
        ),
        (
            "repetitions at i32::MAX",
            SrsStateRow {
                repetitions: max,
                ..valid.clone()
            },
        ),
        (
            "lapses at i32::MAX",
            SrsStateRow {
                lapses: max,
                ..valid
            },
        ),
    ]
}

fn scheduler_vectors() -> Value {
    let scheduler = SrsScheduler::default();
    let initial: Vec<Value> = [
        (1, "2026-10-31T09:15:00Z"),
        (2, "2026-10-31T08:00:00.123456789Z"),
        (3, "2026-10-31T23:59:59.999999999Z"),
        (4, "1970-01-01T00:00:00Z"),
        (5, "1969-12-31T23:59:59.999500Z"),
    ]
    .iter()
    .map(|&(card_id, now)| {
        json!({
            "card_id": card_id,
            "now": now,
            "state": scheduler.initial_state(card_id, instant(now)),
        })
    })
    .collect();

    let sequences = vec![
        wrapper_sequence("default UTC+9", Parameters::default(), 1, 9 * 3600),
        wrapper_sequence("default UTC-5", Parameters::default(), 2, -5 * 3600),
        wrapper_sequence(
            "long term UTC+5:30",
            long_term(Parameters::default()),
            3,
            19_800,
        ),
        wrapper_sequence("long term UTC", long_term(Parameters::default()), 4, 0),
        wrapper_sequence("weights UTC+1", with_weights(), 5, 3600),
        wrapper_sequence(
            "weights long term UTC-9:30",
            long_term(with_weights()),
            6,
            -34_200,
        ),
    ];

    // Explicit cases: a review card rated at and before its last review
    // (the clock was set back), and at instants finer than a millisecond.
    let reviewed = instant("2026-10-31T08:00:00.000Z");
    let day = date("2026-10-31");
    let learned = scheduler
        .review(
            &scheduler.initial_state(1, reviewed),
            ReviewRating::Easy,
            reviewed,
            day,
        )
        .next;
    let learning = scheduler
        .review(
            &scheduler.initial_state(2, reviewed),
            ReviewRating::Good,
            reviewed,
            day,
        )
        .next;
    let mut cases = Vec::new();
    for (state, now) in [
        (&learned, reviewed),
        (&learned, reviewed - Duration::hours(5)),
        (&learned, reviewed - Duration::days(3)),
        (&learned, reviewed - Duration::days(400)),
        (&learned, reviewed - Duration::nanoseconds(1)),
        (&learned, reviewed + Duration::nanoseconds(999_999)),
        (
            &learned,
            learned.due_date + Duration::nanoseconds(1_500_000),
        ),
        (&learning, learning.due_date - Duration::minutes(3)),
        (&learning, reviewed - Duration::minutes(1)),
    ] {
        for given in ReviewRating::ALL {
            cases.push(json!({
                "state": state,
                "now": now,
                "rating": given,
                "session_date": day,
                "outcome": scheduler.review(state, given, now, day),
                "previews": scheduler.preview(state, now),
            }));
        }
    }

    let now = instant("2026-11-09T08:00:00.000Z");
    let invalid: Vec<Value> = invalid_rows()
        .into_iter()
        .map(|(name, row)| {
            json!({
                "name": name,
                "row": row,
                "error": fsrs_card_from_state(&row).err().map(|err| err.to_string()),
                "now": now,
                "previews": scheduler.preview(&row, now),
                "review": scheduler.review(&row, ReviewRating::Good, now, date("2026-11-09")),
            })
        })
        .collect();

    json!({
        "initial": initial,
        "sequences": sequences,
        "cases": cases,
        "invalid": invalid,
    })
}

// ---------------------------------------------------------------------------
// Pure functions: queue, streaks, lessons, fingerprints

fn queue_cases() -> Value {
    let curriculum = scrambled_curriculum();
    let due = vec![
        due_state(9, instant("2026-10-31T12:00:00.000Z")),
        due_state(4, instant("2026-10-31T12:00:00.000Z")),
        due_state(7, instant("2026-10-31T11:59:59.999Z")),
        due_state(1, instant("2026-10-31T12:00:00.001Z")),
        due_state(6, instant("2026-10-31T12:00:00.000Z")),
        due_state(3, instant("2026-10-01T00:00:00.000Z")),
    ];
    let everything: Vec<&str> = curriculum.iter().map(|item| item.hanzi.as_str()).collect();
    let mut reversed = curriculum.clone();
    reversed.reverse();
    type Case<'a> = (
        &'a str,
        &'a [SrsStateRow],
        &'a [CurriculumItem],
        Vec<&'a str>,
        u32,
    );
    let cases: Vec<Case> = vec![
        (
            "reviews first, by due date then card id",
            &due,
            &curriculum,
            vec![],
            2,
        ),
        ("a zero quota is reviews only", &due, &curriculum, vec![], 0),
        ("strict curriculum order", &[], &curriculum, vec![], 6),
        ("reversed input, same order", &[], &reversed, vec![], 3),
        (
            "learned hanzi are skipped",
            &[],
            &curriculum,
            vec!["我", "你好", "nope"],
            3,
        ),
        (
            "quota beyond the curriculum",
            &[],
            &curriculum,
            vec!["我", "是", "好"],
            100,
        ),
        (
            "the largest quota",
            &due[..2],
            &curriculum,
            vec![],
            u32::MAX,
        ),
        ("everything learned", &due[..1], &curriculum, everything, 5),
        ("an empty curriculum", &due, &[], vec![], 10),
        ("nothing at all", &[], &[], vec![], 10),
    ];
    let cases: Vec<Value> = cases
        .into_iter()
        .map(|(name, due, curriculum, learned, quota)| {
            let mut learned: Vec<String> = learned.into_iter().map(str::to_owned).collect();
            learned.sort();
            let set = learned.iter().cloned().collect();
            json!({
                "name": name,
                "due": due,
                "curriculum": curriculum,
                "learned": learned,
                "quota": quota,
                "queue": build_daily_queue_from(due, curriculum, &set, quota),
            })
        })
        .collect();
    Value::Array(cases)
}

fn streak_cases() -> Value {
    let dates =
        |texts: &[&str]| -> Vec<NaiveDate> { texts.iter().map(|text| date(text)).collect() };
    let case = |today: &str, completed: &[&str], frozen: &[&str]| {
        (date(today), dates(completed), dates(frozen))
    };
    let mut cases = vec![
        case("2026-11-20", &[], &[]),
        case("2026-11-20", &["2026-11-20"], &[]),
        case("2026-11-20", &["2026-11-19"], &[]),
        case("2026-11-20", &["2026-11-18", "2026-11-19"], &[]),
        case("2026-11-20", &["2026-11-18"], &[]),
        case(
            "2026-11-20",
            &[
                "2026-11-15",
                "2026-11-16",
                "2026-11-17",
                "2026-11-19",
                "2026-11-20",
            ],
            &[],
        ),
        case(
            "2026-11-20",
            &["2026-11-17", "2026-11-18", "2026-11-20"],
            &["2026-11-19"],
        ),
        case(
            "2026-11-20",
            &["2026-11-16", "2026-11-19"],
            &["2026-11-17", "2026-11-18"],
        ),
        case("2026-11-20", &["2026-11-18"], &["2026-11-19"]),
        case("2026-11-20", &["2026-11-19", "2026-11-20"], &["2026-11-19"]),
        case(
            "2026-11-20",
            &["2026-11-20", "2026-11-21", "2026-11-22"],
            &["2026-11-23"],
        ),
        case(
            "2026-11-20",
            &["2026-10-30", "2026-10-31", "2026-11-01"],
            &[],
        ),
        case(
            "2027-01-01",
            &["2026-12-30", "2026-12-31", "2027-01-01"],
            &[],
        ),
        case(
            "2027-01-02",
            &["2026-12-30", "2026-12-31", "2027-01-01"],
            &[],
        ),
        case(
            "2027-01-03",
            &["2026-12-30", "2026-12-31", "2027-01-01"],
            &[],
        ),
        case("2028-03-01", &["2028-02-28", "2028-02-29"], &[]),
        case("2026-11-20", &[], &["2026-11-19", "2026-11-20"]),
    ];
    // Random months around the end of February in a leap year.
    let mut rng = Mulberry32::new(2028);
    for _ in 0..20 {
        let today = date("2028-03-10") - Duration::days(i64::from(rng.below(5)));
        let mut completed = Vec::new();
        let mut frozen = Vec::new();
        for back in 0..45 {
            let day = today + Duration::days(2 - back);
            match rng.below(6) {
                0..=2 => completed.push(day),
                3 => frozen.push(day),
                _ => {}
            }
        }
        cases.push((today, completed, frozen));
    }
    let cases: Vec<Value> = cases
        .into_iter()
        .map(|(today, completed, frozen)| {
            let completed_set = completed.iter().copied().collect();
            let frozen_set = frozen.iter().copied().collect();
            let (current, longest) = compute_streak(&completed_set, &frozen_set, today);
            let mut completed = completed;
            completed.sort();
            let mut frozen = frozen;
            frozen.sort();
            json!({
                "today": today,
                "completed": completed,
                "frozen": frozen,
                "current": current,
                "longest": longest,
            })
        })
        .collect();
    Value::Array(cases)
}

fn stored(id: i64, card: Card, origin: CaptureOrigin) -> StoredCapture {
    StoredCapture {
        id,
        card,
        origin,
        synced_to_anki: false,
    }
}

fn lesson_cases() -> Value {
    let capture = CaptureOrigin::Capture;
    let at = |minutes: i64| -> String {
        (instant("2026-10-01T08:00:00Z") + Duration::minutes(minutes)).to_rfc3339()
    };
    let english = |front: &str, sentence: &str, translation: &str, minutes: i64| {
        captured_at(
            with_example(plain(front, "Bedeutung", "en", "de"), sentence, translation),
            &at(minutes),
        )
    };
    let captures = vec![
        stored(
            3,
            english(
                "serendipity",
                "It was pure Serendipity.",
                "Es war reiner Zufall.",
                5,
            ),
            capture,
        ),
        stored(
            1,
            english("cat", "Concatenate the cat!", "Verkette die Katze!", 1),
            capture,
        ),
        stored(2, english("run", "He is running.", "Er rennt.", 1), capture),
        stored(
            4,
            captured_at(
                with_example(
                    chinese("学习", "lernen", "de", "xué xí"),
                    "我们学习学习。",
                    "Wir lernen.",
                ),
                &at(2),
            ),
            capture,
        ),
        stored(
            5,
            captured_at(
                with_example(
                    chinese("中文", "Chinesisch", "de", "zhōng wén"),
                    "我学习中文。",
                    "Ich lerne Chinesisch.",
                ),
                &at(3),
            ),
            CaptureOrigin::Curriculum,
        ),
        stored(
            6,
            english("dog", "The dog sleeps.", "Der Hund schläft.", 4),
            CaptureOrigin::AnkiImport,
        ),
        stored(
            7,
            captured_at(
                with_example(
                    plain("Straße", "street", "de", "en"),
                    "STRAẞE und Straße",
                    "street and street",
                ),
                &at(6),
            ),
            capture,
        ),
        stored(
            8,
            english("word", "A word without translation.", "  ", 7),
            capture,
        ),
        stored(
            9,
            english(
                "absent",
                "This sentence lacks it.",
                "Dieser Satz hat es nicht.",
                8,
            ),
            capture,
        ),
        stored(
            10,
            captured_at(
                with_example(
                    plain("  Haus  ", " house ", "de", "en"),
                    "  Das Haus ist alt.  ",
                    " The house is old. ",
                ),
                &at(9),
            ),
            capture,
        ),
        stored(
            11,
            captured_at(
                with_example(
                    plain("istanbul", "Istanbul", "tr", "de"),
                    "İstanbul ist groß.",
                    "Istanbul ist groß.",
                ),
                &at(10),
            ),
            capture,
        ),
        stored(
            12,
            captured_at(
                with_example(
                    plain("chat", "Katze", "fr", "de"),
                    "Le chat dort.",
                    "Die Katze schläft.",
                ),
                &at(11),
            ),
            capture,
        ),
        stored(
            13,
            english(
                "\u{85}book\u{85}",
                "I read a book.",
                "Ich lese ein Buch.",
                12,
            ),
            capture,
        ),
        stored(
            14,
            english("cat", "The cat, the CAT.", "Die Katze, die KATZE.", 1),
            capture,
        ),
        stored(
            15,
            captured_at(
                with_example(chinese("你好", "hallo", "de", "  "), "你好！", "Hallo!"),
                &at(13),
            ),
            capture,
        ),
        stored(
            16,
            english("end", "This is the end", "Das ist das Ende", 14),
            capture,
        ),
        stored(
            17,
            english("é", "Caf\u{e9} au lait, cafe\u{301}.", "Milchkaffee.", 15),
            capture,
        ),
    ];
    let cases: Vec<Value> = [0, 1, 2, 3, 100]
        .iter()
        .map(|&size| {
            json!({
                "cards_per_lesson": size,
                "lessons": build_lessons(&captures, size),
            })
        })
        .collect();
    json!({ "captures": captures, "cases": cases })
}

fn fingerprint_cases() -> BoxResult<Value> {
    let mut negative = curriculum_item("负", -3);
    negative.frequency = Some(-1_000_000_007);
    negative.traditional = Some(String::new());
    let mut full = curriculum_item("学习", 9_007_199_254);
    full.traditional = Some("學習".into());
    full.pos = Some("v".into());
    full.cloze_sentence = Some("我__中文。".into());
    full.example_sentence_source = Some("我学习中文。".into());
    full.example_sentence_pinyin = Some("wǒ xué xí zhōng wén".into());
    full.example_sentence_target = Some("I study Chinese.".into());
    full.vocab_audio_path = Some("a.mp3".into());
    full.sentence_audio_path = Some("s.mp3".into());
    full.image_path = Some("i.png".into());
    full.hsk_level = 7;
    full.frequency = None;
    let mut reversed = scrambled_curriculum();
    reversed.reverse();
    let sets: Vec<(&str, Vec<CurriculumItem>)> = vec![
        ("empty", vec![]),
        ("one item", vec![curriculum_item("学习", 1)]),
        ("scrambled", scrambled_curriculum()),
        ("scrambled, reversed", reversed),
        ("negative numbers and an empty traditional", vec![negative]),
        ("every field set", vec![full]),
    ];
    let cases: Vec<Value> = sets
        .into_iter()
        .map(|(name, items)| {
            json!({ "name": name, "items": items, "fingerprint": curriculum_fingerprint(&items) })
        })
        .collect();
    let bundled = bundled_curriculum()?;
    Ok(json!({
        "cases": cases,
        "bundled": { "count": bundled.len(), "fingerprint": curriculum_fingerprint(&bundled) },
    }))
}

// ---------------------------------------------------------------------------
// Exercises over a fixed pool

/// One capture of the exercise fixture.
struct Word {
    card: Card,
    origin: CaptureOrigin,
    learned: bool,
}

fn word(card: Card, learned: bool) -> Word {
    Word {
        card,
        origin: CaptureOrigin::Capture,
        learned,
    }
}

/// Chinese and Latin cards in several language pairs, with traps for every
/// rule: translations and sentences with unlearned hanzi, homophones,
/// duplicates that differ in case only, blank sides, a Chinese card without
/// script metadata, and captures that are not learned.
fn exercise_words() -> Vec<Word> {
    let mut chinese_no_metadata = plain("你好", "hallo", "zh-CN", "de");
    chinese_no_metadata.example_source = "你好，老师！".into();
    chinese_no_metadata.example_target = "Hallo, Herr Lehrer!".into();
    vec![
        word(
            with_example(
                chinese("学习", "lernen", "de", "xué xí"),
                "我们学习中文。",
                "Wir lernen Chinesisch.",
            ),
            true,
        ),
        word(
            with_audio(with_example(
                chinese("学生", "Schüler", "de", "xué sheng"),
                "学生是我们的朋友。",
                "Die Schüler sind unsere Freunde.",
            )),
            false,
        ),
        word(
            with_audio(with_example(
                chinese("中文", "Chinesisch", "de", "zhōng wén"),
                "我们的中文老师很好。",
                "Unser Chinesischlehrer ist gut.",
            )),
            true,
        ),
        Word {
            origin: CaptureOrigin::Curriculum,
            ..word(
                with_audio(with_example(
                    chinese("我们", "wir", "de", "wǒ men"),
                    "我们是朋友。",
                    "Wir sind Freunde.",
                )),
                true,
            )
        },
        word(
            with_example(
                chinese("朋友", "Freund", "de", "péng you"),
                "他是我的好朋友。",
                "Er ist mein guter Freund.",
            ),
            true,
        ),
        word(
            with_audio(with_example(
                chinese("老师", "Lehrer", "de", "lǎo shī"),
                "老师今天很好。",
                "Dem Lehrer geht es heute gut.",
            )),
            true,
        ),
        word(
            with_example(
                chinese("今天", "heute", "de", "jīn tiān"),
                "今天天气很好。",
                "Heute ist das Wetter gut.",
            ),
            true,
        ),
        word(
            with_example(
                chinese("是", "sein", "de", "shì"),
                "这是我的书。",
                "Das ist mein Buch.",
            ),
            true,
        ),
        word(chinese("事", "Sache", "de", "shì"), true),
        word(
            with_example(chinese("很", "sehr", "de", "hěn"), "很好。", "Sehr gut."),
            true,
        ),
        word(chinese("好", "gut", "de", "hǎo"), true),
        word(
            with_audio(with_example(
                chinese("你", "du", "de", "nǐ"),
                "你好吗？",
                "Wie geht es dir?",
            )),
            true,
        ),
        word(
            with_example(
                chinese("他", "er", "de", "tā"),
                "他很好。",
                "Ihm geht es gut.",
            ),
            false,
        ),
        word(
            with_example(
                chinese("中文", "Chinese", "en", "zhōng wén"),
                "我们学习中文。",
                "We study Chinese.",
            ),
            true,
        ),
        word(
            with_example(
                chinese("喜欢", "to like", "en", "xǐhuan"),
                "我们喜欢学习中文。",
                "We like to study Chinese.",
            ),
            true,
        ),
        word(chinese("中国", "", "de", "zhōng guó"), true),
        word(chinese_no_metadata, true),
        word(
            with_audio(with_example(
                plain("house", "Haus", "en", "de"),
                "The house is big.",
                "Das Haus ist groß.",
            )),
            true,
        ),
        word(
            with_audio(with_example(
                plain("serendipity", "glücklicher Zufall", "en", "de"),
                "It was pure serendipity.",
                "Es war reiner Zufall.",
            )),
            true,
        ),
        word(
            with_example(
                plain("book", "书", "en", "zh-CN"),
                "I read a book.",
                "我读书。",
            ),
            true,
        ),
        word(
            with_example(
                plain("Haus", "house", "de", "en"),
                "Das Haus ist alt.",
                "The house is old.",
            ),
            true,
        ),
        word(
            with_example(
                plain("maison", "Haus", "fr", "de"),
                "La maison est grande.",
                "Das Haus ist groß.",
            ),
            true,
        ),
        word(
            with_example(
                plain("run", "laufen", "en", "de"),
                "He is running fast.",
                "Er läuft schnell.",
            ),
            true,
        ),
        word(plain("HOUSE", "haus", "en", "de"), true),
        word(
            with_example(
                plain("cat", "Katze", "en", "de"),
                "Concatenate the cat.",
                "Verkette die Katze.",
            ),
            true,
        ),
        Word {
            origin: CaptureOrigin::AnkiImport,
            ..word(
                with_example(
                    plain("school", "Schule", "en", "de"),
                    "The school is old.",
                    "Die Schule ist alt.",
                ),
                false,
            )
        },
        // Fronts that are no one-tile word (a pattern, a phrase) must never
        // become tiles; a dictionary word may start with Latin letters; and a
        // Latin run inside a Chinese sentence is one tile.
        word(chinese("一…就…", "sobald", "de", "yī … jiù …"), true),
        word(chinese("你 好", "hallo", "de", "nǐ hǎo"), true),
        word(
            with_example(
                chinese("AA制", "getrennt zahlen", "de", "AA zhì"),
                "我们AA制。",
                "Wir zahlen getrennt.",
            ),
            true,
        ),
        word(
            with_example(
                chinese("用", "benutzen", "de", "yòng"),
                "我用iPhone学习中文。",
                "Ich lerne Chinesisch mit dem iPhone.",
            ),
            true,
        ),
        // Every side padded with whitespace: prompts, options, the
        // dictionary word and the sentence prompt are all trimmed.
        word(
            with_example(
                chinese(" 国家 ", " Land ", "de", " guó jiā "),
                " 我们国家很好。 ",
                "  Unser Land ist gut.  ",
            ),
            true,
        ),
        // A decomposed accent is part of its word, not punctuation.
        word(
            with_example(
                plain("cafe\u{301}", "Kaffee", "fr", "de"),
                "Un cafe\u{301}, s'il vous plaît.",
                "Einen Kaffee, bitte.",
            ),
            true,
        ),
        // A sentence of one tile is nothing to put in order.
        word(
            with_example(
                chinese("谢谢", "danke", "de", "xiè xie"),
                "谢谢！",
                "Danke!",
            ),
            true,
        ),
    ]
}

fn learned_state(card_id: i64) -> SrsStateRow {
    SrsStateRow {
        card_id,
        algorithm: "fsrs".into(),
        stability: Some(3.2),
        difficulty: Some(5.1),
        interval_days: 3,
        repetitions: 2,
        lapses: 0,
        fsrs_state: "review".into(),
        elapsed_days: 1,
        due_date: instant("2026-11-02T08:00:00.000Z"),
        last_reviewed_at: Some(instant("2026-10-30T08:00:00.000Z")),
        last_rating: Some(3),
    }
}

fn exercise_vectors() -> BoxResult<Value> {
    let store = Store::open_in_memory()?;
    let mut captures = Vec::new();
    let mut learned_ids = Vec::new();
    for word in exercise_words() {
        let id = store.insert_capture(&word.card, word.origin, false)?;
        if word.learned {
            store.upsert_srs_state(&learned_state(id))?;
            learned_ids.push(id);
        }
        captures.push(json!({
            "card": word.card,
            "origin": word.origin,
            "learned": word.learned,
            "id": id,
        }));
    }
    let pool = LearnedVocabularyPool::load(&store)?;
    let mut hanzi: Vec<String> = pool
        .learned_hanzi_chars()
        .iter()
        .map(char::to_string)
        .collect();
    hanzi.sort();

    let seeds = [0, 1, 2, 42, 0xDEAD_BEEF, u32::MAX];
    let mut cases = Vec::new();
    let mut samples: Vec<Exercise> = Vec::new();
    for &card_id in &learned_ids {
        for kind in ExerciseKind::ALL {
            for seed in seeds {
                let generated = generate_exercise(&pool, card_id, kind, seed);
                if let Ok(exercise) = &generated {
                    if seed == 1 && samples.iter().all(|sample| sample.kind() != kind) {
                        samples.push(exercise.clone());
                    }
                }
                cases.push(json!({
                    "card_id": card_id,
                    "kind": kind,
                    "seed": seed,
                    "result": outcome(generated),
                }));
            }
        }
    }
    for card_id in (1..=captures.len() as i64)
        .filter(|id| !learned_ids.contains(id))
        .chain([0, 999])
    {
        for kind in ExerciseKind::ALL {
            cases.push(json!({
                "card_id": card_id,
                "kind": kind,
                "seed": 3,
                "result": outcome(generate_exercise(&pool, card_id, kind, 3)),
            }));
        }
    }

    Ok(json!({
        "captures": captures,
        "pool": pool.entries(),
        "learned_hanzi": hanzi,
        "cases": cases,
        "answers": answer_cases(&samples),
    }))
}

/// Answers to one exercise of each kind: right, wrong, out of range, the
/// wrong shape, tiles used twice or unknown, and equal-text tiles swapped.
fn answer_cases(samples: &[Exercise]) -> Value {
    let mut cases = Vec::new();
    let mut add = |exercise: &Exercise, answer: ExerciseAnswer| {
        cases.push(json!({
            "exercise": exercise,
            "answer": answer,
            "correct": check_answer(exercise, &answer),
        }));
    };
    for exercise in samples {
        match exercise {
            Exercise::MultipleChoice(e) => {
                choice_answers(exercise, e.correct_index, e.options.len(), &mut add)
            }
            Exercise::ListeningChoice(e) => {
                choice_answers(exercise, e.correct_index, e.options.len(), &mut add)
            }
            Exercise::SentenceBuilder(e) => {
                order_answers(exercise, &e.solution, e.tiles.len(), &mut add)
            }
            Exercise::ListeningSentence(e) => {
                order_answers(exercise, &e.solution, e.tiles.len(), &mut add)
            }
        }
    }
    // A tampered exercise whose correct index points past its options: even
    // the "correct" index is wrong.
    if let Some(Exercise::MultipleChoice(sample)) = samples
        .iter()
        .find(|e| matches!(e, Exercise::MultipleChoice(_)))
    {
        let mut tampered = sample.clone();
        tampered.options.truncate(tampered.correct_index);
        let index = tampered.correct_index;
        add(
            &Exercise::MultipleChoice(tampered),
            ExerciseAnswer::Choice { index },
        );
    }
    // Equal texts are interchangeable: a sentence that repeats a tile.
    if let Some(Exercise::SentenceBuilder(sample)) = samples
        .iter()
        .find(|e| matches!(e, Exercise::SentenceBuilder(_)))
    {
        let mut twice = sample.clone();
        twice.tiles = vec![
            sample_tile(0, "好"),
            sample_tile(1, "很"),
            sample_tile(2, "好"),
        ];
        twice.solution = vec![1, 0, 2];
        let exercise = Exercise::SentenceBuilder(twice);
        for order in [
            vec![1, 0, 2],
            vec![1, 2, 0],
            vec![0, 1, 2],
            vec![1, 0, 0],
            vec![1, 2],
        ] {
            add(&exercise, ExerciseAnswer::Order { tile_ids: order });
        }
    }
    Value::Array(cases)
}

fn sample_tile(id: u32, text: &str) -> vocab_core::quiz::Tile {
    vocab_core::quiz::Tile {
        id,
        text: text.into(),
    }
}

fn choice_answers(
    exercise: &Exercise,
    correct: usize,
    options: usize,
    add: &mut impl FnMut(&Exercise, ExerciseAnswer),
) {
    for index in [correct, (correct + 1) % options, options, options + 7] {
        add(exercise, ExerciseAnswer::Choice { index });
    }
    add(exercise, ExerciseAnswer::Order { tile_ids: vec![0] });
}

fn order_answers(
    exercise: &Exercise,
    solution: &[u32],
    tiles: usize,
    add: &mut impl FnMut(&Exercise, ExerciseAnswer),
) {
    add(
        exercise,
        ExerciseAnswer::Order {
            tile_ids: solution.to_vec(),
        },
    );
    let mut reversed = solution.to_vec();
    reversed.reverse();
    add(exercise, ExerciseAnswer::Order { tile_ids: reversed });
    add(
        exercise,
        ExerciseAnswer::Order {
            tile_ids: solution[..solution.len() - 1].to_vec(),
        },
    );
    let mut longer = solution.to_vec();
    longer.push(solution[0]);
    add(exercise, ExerciseAnswer::Order { tile_ids: longer });
    let mut unknown = solution.to_vec();
    unknown[0] = tiles as u32 + 3;
    add(exercise, ExerciseAnswer::Order { tile_ids: unknown });
    let distractor = (0..tiles as u32).find(|id| !solution.contains(id));
    if let Some(distractor) = distractor {
        let mut swapped = solution.to_vec();
        swapped[0] = distractor;
        add(exercise, ExerciseAnswer::Order { tile_ids: swapped });
    }
    add(exercise, ExerciseAnswer::Choice { index: 0 });
}

// ---------------------------------------------------------------------------
// Store-backed scenarios

/// Replays core functions against one in-memory store and records every call
/// with its arguments and result, for the TypeScript side to replay against
/// its own store. Op names are the Rust function names.
struct Runner {
    name: String,
    store: Store,
    clock: FixedClock,
    scheduler: SrsScheduler,
    offset_seconds: i32,
    start: DateTime<Utc>,
    steps: Vec<Value>,
}

impl Runner {
    fn new(name: &str, start: DateTime<Utc>, offset_seconds: i32) -> Self {
        Self {
            name: name.to_owned(),
            store: Store::open_in_memory().expect("in-memory store"),
            clock: FixedClock::new(start, offset(offset_seconds)),
            scheduler: SrsScheduler::default(),
            offset_seconds,
            start,
            steps: Vec::new(),
        }
    }

    fn finish(self) -> Value {
        json!({
            "name": self.name,
            "start": self.start,
            "offset_seconds": self.offset_seconds,
            "steps": self.steps,
        })
    }

    fn record(&mut self, mut step: Value, result: Value) {
        step["result"] = result;
        self.steps.push(step);
    }

    fn local(&self, text: &str) -> DateTime<Utc> {
        local(offset(self.offset_seconds), text)
    }

    fn set_clock(&mut self, now: DateTime<Utc>) {
        self.clock.set(now);
        self.steps.push(json!({ "op": "set_clock", "now": now }));
    }

    fn set_local(&mut self, text: &str) {
        let now = self.local(text);
        self.set_clock(now);
    }

    fn advance(&mut self, by: Duration) {
        let now = self.clock.now() + by;
        self.set_clock(now);
    }

    fn replace_curriculum(&mut self, items: &[CurriculumItem]) {
        let result = self.store.replace_curriculum(items);
        self.record(
            json!({ "op": "replace_curriculum", "items": items }),
            outcome(result),
        );
    }

    fn ensure_curriculum_seeded(&mut self, items: &[CurriculumItem]) {
        let result = self.store.ensure_curriculum_seeded(items);
        self.record(
            json!({ "op": "ensure_curriculum_seeded", "items": items }),
            outcome(result),
        );
        let count = self.store.curriculum_count();
        self.record(json!({ "op": "curriculum_count" }), outcome(count));
    }

    fn insert_capture(&mut self, card: &Card, origin: CaptureOrigin) -> i64 {
        let result = self.store.insert_capture(card, origin, false);
        let id = *result.as_ref().expect("fixture captures insert");
        self.record(
            json!({ "op": "insert_capture", "card": card, "origin": origin }),
            outcome(result),
        );
        id
    }

    fn upsert_srs_state(&mut self, row: &SrsStateRow) {
        let result = self.store.upsert_srs_state(row);
        self.record(
            json!({ "op": "upsert_srs_state", "row": row }),
            outcome(result),
        );
    }

    /// A word introduced at `at` and never rated since: due, waiting for the
    /// first review, which counts as a new item.
    fn introduced_at(&mut self, card_id: i64, at: DateTime<Utc>) {
        let row = self.scheduler.initial_state(card_id, at);
        self.upsert_srs_state(&row);
    }

    fn upsert_daily_session(&mut self, row: &DailySessionRow) {
        let result = self.store.upsert_daily_session(row);
        self.record(
            json!({ "op": "upsert_daily_session", "row": row }),
            outcome(result),
        );
    }

    fn complete_day(&mut self, day: &str) {
        let row = DailySessionRow {
            session_date: date(day),
            requested_new_count: 3,
            reviews_due_count: 5,
            reviews_completed: 5,
            new_items_completed: 3,
            completed_at: Some(instant("2026-11-01T00:00:00.000Z")),
        };
        self.upsert_daily_session(&row);
    }

    fn set_state(&mut self, key: &str, value: &str) {
        let result = self.store.set_state(key, value);
        self.record(
            json!({ "op": "set_state", "key": key, "value": value }),
            outcome(result),
        );
    }

    fn get_state(&mut self, key: &str) {
        let result = self.store.get_state(key);
        self.record(json!({ "op": "get_state", "key": key }), outcome(result));
    }

    fn insert_streak_freeze(&mut self, day: &str) {
        let result = self.store.insert_streak_freeze(date(day));
        self.record(
            json!({ "op": "insert_streak_freeze", "date": day }),
            outcome(result),
        );
    }

    fn build_daily_queue(&mut self, quota: u32) {
        let result = build_daily_queue(&self.store, quota, &self.clock);
        self.record(
            json!({ "op": "build_daily_queue", "quota": quota }),
            outcome(result),
        );
    }

    fn introduce_new_item(&mut self, hanzi: &str, target_lang: &str) -> Option<i64> {
        let result = introduce_new_item(
            &self.store,
            &self.scheduler,
            hanzi,
            target_lang,
            &self.clock,
        );
        let id = result.as_ref().ok().copied();
        self.record(
            json!({ "op": "introduce_new_item", "hanzi": hanzi, "target_lang": target_lang }),
            outcome(result),
        );
        id
    }

    fn submit_review(&mut self, card_id: i64, given: u8) {
        let result = submit_review(
            &self.store,
            &self.scheduler,
            card_id,
            rating(given),
            &self.clock,
        );
        self.record(
            json!({ "op": "submit_review", "card_id": card_id, "rating": given }),
            outcome(result),
        );
    }

    fn preview(&mut self, card_id: i64) {
        let result = self
            .store
            .get_srs_state(card_id)
            .map(|state| state.map(|state| self.scheduler.preview(&state, self.clock.now())));
        self.record(
            json!({ "op": "preview", "card_id": card_id }),
            outcome(result),
        );
    }

    fn get_capture(&mut self, id: i64) {
        let result = self.store.get_capture(id);
        self.record(json!({ "op": "get_capture", "id": id }), outcome(result));
    }

    fn get_srs_state(&mut self, card_id: i64) {
        let result = self.store.get_srs_state(card_id);
        self.record(
            json!({ "op": "get_srs_state", "card_id": card_id }),
            outcome(result),
        );
    }

    fn review_log_for_card(&mut self, card_id: i64) {
        let result = self.store.review_log_for_card(card_id);
        self.record(
            json!({ "op": "review_log_for_card", "card_id": card_id }),
            outcome(result),
        );
    }

    fn list_captures(&mut self) {
        let result = self.store.list_captures(None);
        self.record(json!({ "op": "list_captures" }), outcome(result));
    }

    fn learned_chinese_fronts(&mut self) {
        let result = self.store.learned_chinese_fronts().map(|fronts| {
            let mut fronts: Vec<String> = fronts.into_iter().collect();
            fronts.sort();
            fronts
        });
        self.record(json!({ "op": "learned_chinese_fronts" }), outcome(result));
    }

    fn delete_capture(&mut self, id: i64) {
        let result = self.store.delete_capture(id);
        self.record(json!({ "op": "delete_capture", "id": id }), outcome(result));
    }

    fn daily_prompt_state(&mut self) {
        let result = daily_prompt_state(&self.store, &self.clock);
        self.record(json!({ "op": "daily_prompt_state" }), outcome(result));
    }

    fn submit_daily_quota(&mut self, new_count: u32) {
        let result = submit_daily_quota(&self.store, new_count, &self.clock);
        self.record(
            json!({ "op": "submit_daily_quota", "new_count": new_count }),
            outcome(result),
        );
    }

    fn dismiss_daily_prompt(&mut self) {
        let result = dismiss_daily_prompt(&self.store, &self.clock);
        self.record(json!({ "op": "dismiss_daily_prompt" }), outcome(result));
    }

    fn reopen_daily_prompt(&mut self) {
        let result = reopen_daily_prompt(&self.store, &self.clock);
        self.record(json!({ "op": "reopen_daily_prompt" }), outcome(result));
    }

    fn record_session_progress(&mut self, kind: SessionProgress) {
        let result = record_session_progress(&self.store, kind, &self.clock);
        self.record(
            json!({ "op": "record_session_progress", "kind": kind }),
            outcome(result),
        );
    }

    fn award_xp(&mut self, amount: i64, why: &str) {
        let result = award_xp(&self.store, amount, why, &self.clock);
        self.record(
            json!({ "op": "award_xp", "amount": amount, "reason": why }),
            outcome(result),
        );
    }

    fn apply_streak_freezes(&mut self) {
        let result = apply_streak_freezes(&self.store, &self.clock);
        self.record(json!({ "op": "apply_streak_freezes" }), outcome(result));
    }

    fn on_session_completed(&mut self) {
        let result = on_session_completed(&self.store, &self.clock);
        self.record(json!({ "op": "on_session_completed" }), outcome(result));
    }

    fn stats_summary(&mut self) {
        let result = stats_summary(&self.store, &self.clock);
        self.record(json!({ "op": "stats_summary" }), outcome(result));
    }

    fn load_pool(&mut self) {
        let result = LearnedVocabularyPool::load(&self.store).map(|pool| {
            let mut hanzi: Vec<String> = pool
                .learned_hanzi_chars()
                .iter()
                .map(char::to_string)
                .collect();
            hanzi.sort();
            json!({ "entries": pool.entries(), "learned_hanzi": hanzi })
        });
        self.record(json!({ "op": "load_pool" }), outcome(result));
    }

    fn generate_exercise(&mut self, card_id: i64, kind: ExerciseKind, seed: u32) {
        let result = LearnedVocabularyPool::load(&self.store)
            .map_err(|err| err.to_string())
            .and_then(|pool| {
                generate_exercise(&pool, card_id, kind, seed).map_err(|err| err.to_string())
            });
        self.record(
            json!({ "op": "generate_exercise", "card_id": card_id, "kind": kind, "seed": seed }),
            outcome(result),
        );
    }

    fn lessons_from_store(&mut self, cards_per_lesson: usize) {
        let result = lessons_from_store(&self.store, cards_per_lesson);
        self.record(
            json!({ "op": "lessons_from_store", "cards_per_lesson": cards_per_lesson }),
            outcome(result),
        );
    }
}

fn scenarios() -> Value {
    let mut all = Vec::new();
    for hours in [1, -5] {
        all.push(daily_scenario(hours));
    }
    all.push(empty_days_scenario());
    for offset_seconds in [9 * 3600, -5 * 3600, 19_800, -34_200] {
        all.push(queue_boundaries_scenario(offset_seconds));
    }
    all.push(queue_learning_scenario());
    all.push(gamification_scenario());
    all.push(freezes_at_completion_scenario());
    all.push(pool_and_lessons_scenario());
    all.push(seeding_scenario());
    Value::Array(all)
}

/// docs/architecture.md § 7.1's daily prompt: once per local day, dismiss
/// means reviews only, reopening keeps the progress, and a session completes
/// exactly when both targets are met after some work.
fn daily_scenario(hours: i32) -> Value {
    let mut run = Runner::new(
        &format!("daily prompt at UTC{hours:+}"),
        local(offset(hours * 3600), "2026-10-31 07:00:00.000"),
        hours * 3600,
    );
    // Two reviews due on Oct 31 (one this evening) and one on Nov 1, and a
    // word introduced on Oct 30 and never rated: due, but its first rating
    // is a new item, so it is no review target.
    let mut ids = Vec::new();
    for (index, due) in [
        "2026-10-31 06:00:00.000",
        "2026-10-31 22:00:00.000",
        "2026-11-01 06:00:00.000",
        "2026-10-30 21:00:00.000",
    ]
    .iter()
    .enumerate()
    {
        let card = Card::new(
            format!("word {index}"),
            "Bedeutung",
            "en",
            "de",
            run.local("2026-10-01 00:00:00.000"),
        );
        let id = run.insert_capture(&card, CaptureOrigin::Capture);
        let due = run.local(due);
        if index < 3 {
            run.upsert_srs_state(&due_state(id, due));
        } else {
            run.introduced_at(id, due);
        }
        ids.push(id);
    }
    run.daily_prompt_state();
    run.reopen_daily_prompt();
    run.daily_prompt_state();
    run.submit_daily_quota(2);
    run.daily_prompt_state();
    run.dismiss_daily_prompt();
    for kind in [
        SessionProgress::Review,
        SessionProgress::NewItem,
        SessionProgress::Review,
        SessionProgress::NewItem,
    ] {
        run.advance(Duration::minutes(3) + Duration::nanoseconds(123_456));
        run.record_session_progress(kind);
    }
    run.advance(Duration::minutes(30));
    run.record_session_progress(SessionProgress::Review);
    run.reopen_daily_prompt();
    run.daily_prompt_state();
    run.submit_daily_quota(5);
    run.dismiss_daily_prompt();
    run.daily_prompt_state();

    // Past midnight, still reviewing: the new day opens with quota 0.
    run.set_local("2026-11-01 00:05:00.000");
    run.record_session_progress(SessionProgress::Review);
    run.daily_prompt_state();
    run.submit_review(ids[0], 4);
    run.submit_daily_quota(4);
    run.reopen_daily_prompt();
    run.submit_daily_quota(6);

    // A stored prompt date that does not parse counts as never prompted.
    run.set_local("2026-11-02 09:00:00.000");
    run.set_state(LAST_PROMPTED_KEY, "31.10.2026");
    run.daily_prompt_state();
    run.submit_daily_quota(3);
    run.get_state(LAST_PROMPTED_KEY);
    run.record_session_progress(SessionProgress::NewItem);
    // Lowering the quota to what is done completes the session.
    run.advance(Duration::minutes(10));
    run.reopen_daily_prompt();
    run.submit_daily_quota(1);
    run.submit_daily_quota(0);
    // A stored prompt date is read as leniently as chrono parses it: this
    // one is today.
    run.set_state(LAST_PROMPTED_KEY, " 2026-11- 2");
    run.daily_prompt_state();
    run.finish()
}

/// A day with nothing to do never completes, however it was set up.
fn empty_days_scenario() -> Value {
    let mut run = Runner::new(
        "days with nothing to do at UTC-5",
        local(offset(-5 * 3600), "2026-10-31 07:00:00.000"),
        -5 * 3600,
    );
    run.dismiss_daily_prompt();
    run.daily_prompt_state();
    run.set_local("2026-11-01 08:00:00.000");
    run.submit_daily_quota(0);
    run.set_local("2026-11-02 08:00:00.000");
    run.submit_daily_quota(1);
    run.record_session_progress(SessionProgress::NewItem);
    run.finish()
}

/// "Due today" ends at local midnight, in offsets on both sides of UTC and
/// at half and quarter hours.
fn queue_boundaries_scenario(offset_seconds: i32) -> Value {
    let mut run = Runner::new(
        &format!("queue day boundaries at {offset_seconds} s"),
        local(offset(offset_seconds), "2026-10-31 08:00:00.000"),
        offset_seconds,
    );
    let dues = [
        "2026-09-01 12:00:00.000",
        "2026-10-31 00:00:00.000",
        "2026-10-31 08:00:00.000",
        "2026-10-31 21:30:00.000",
        "2026-10-31 23:59:59.999",
        "2026-11-01 00:00:00.000",
        "2026-11-01 07:00:00.000",
    ];
    for (index, due) in dues.iter().enumerate() {
        let card = chinese(&format!("字{index}"), "Zeichen", "de", "zì");
        let id = run.insert_capture(&card, CaptureOrigin::Capture);
        let due = run.local(due);
        run.upsert_srs_state(&due_state(id, due));
    }
    run.replace_curriculum(&scrambled_curriculum());
    run.build_daily_queue(0);
    run.build_daily_queue(2);
    run.stats_summary();
    run.set_local("2026-10-31 23:59:59.999");
    run.build_daily_queue(0);
    run.set_local("2026-11-01 00:00:00.000");
    run.build_daily_queue(0);
    run.stats_summary();
    run.finish()
}

/// New words: learned fronts are skipped, an Automator capture is adopted,
/// the meaning language falls back to English, and the example translation
/// is copied for English cards only.
fn queue_learning_scenario() -> Value {
    let mut run = Runner::new(
        "queue, introducing and reviewing",
        instant("2026-10-31T08:00:00.000456789Z"),
        3600,
    );
    let mut items = scrambled_curriculum();
    for item in &mut items {
        match item.hanzi.as_str() {
            "学习" => {
                item.meaning_de = Some("lernen".into());
                item.meaning_en = Some("to study".into());
                item.example_sentence_source = Some("我学习中文。".into());
                item.example_sentence_target = Some("I study Chinese.".into());
                item.traditional = Some("學習".into());
            }
            "你好" => {
                item.meaning_de = None;
                item.meaning_en = Some("hello".into());
                item.example_sentence_source = Some("你好！".into());
                item.example_sentence_target = Some("Hello!".into());
            }
            "中文" => item.meaning_de = Some("  ".into()),
            "是" => {
                item.meaning_de = Some(String::new());
                item.meaning_en = Some(" ".into());
                item.example_sentence_target = Some("It is.".into());
            }
            "好" => {
                item.meaning_en = None;
                item.example_sentence_source = Some("好！".into());
                item.example_sentence_target = Some("Good!".into());
            }
            _ => {}
        }
    }
    // Introduced with language codes that need normalizing; 朋友 has no
    // meaning at all, so its card keeps the (normalized) language asked for.
    items.push(curriculum_item("老师", 40));
    let mut no_meaning = curriculum_item("朋友", 41);
    no_meaning.meaning_en = None;
    no_meaning.meaning_de = None;
    no_meaning.example_sentence_target = Some("A friend.".into());
    items.push(no_meaning);
    run.replace_curriculum(&items);
    let captured = instant("2026-10-01T00:00:00Z");
    let mut learned = chinese("我", "ich", "de", "wǒ");
    learned.captured_at = captured;
    let learned = run.insert_capture(&learned, CaptureOrigin::Capture);
    run.upsert_srs_state(&due_state(learned, instant("2026-10-31T10:00:00.000Z")));
    let english = run.insert_capture(
        &captured_at(plain("是", "is", "en", "de"), "2026-10-01T00:00:00Z"),
        CaptureOrigin::Capture,
    );
    run.upsert_srs_state(&due_state(english, instant("2026-10-31T06:00:00.000Z")));
    let automator = run.insert_capture(
        &captured_at(
            chinese("学习", "Bedeutung", "de", "xué xí"),
            "2026-10-01T00:00:00Z",
        ),
        CaptureOrigin::Capture,
    );
    run.insert_capture(
        &captured_at(
            chinese("学习", "Duplikat", "de", "xué xí"),
            "2026-10-02T00:00:00Z",
        ),
        CaptureOrigin::Capture,
    );

    run.build_daily_queue(2);
    run.build_daily_queue(0);
    run.build_daily_queue(99);
    run.learned_chinese_fronts();
    run.introduce_new_item("学习", "de");
    run.get_srs_state(automator);
    run.get_capture(automator);
    run.build_daily_queue(5);
    for (hanzi, target_lang) in [
        ("你好", "de"),
        ("中文", "de"),
        ("是", "de"),
        ("好", "en"),
        ("学习", "en"),
        ("你好", "DE"),
        ("老师", " DE "),
        ("朋友", "FR "),
    ] {
        if let Some(id) = run.introduce_new_item(hanzi, target_lang) {
            run.get_capture(id);
        }
    }
    for missing in ["没有", " 学习", ""] {
        run.introduce_new_item(missing, "de");
    }
    run.list_captures();
    run.learned_chinese_fronts();
    run.build_daily_queue(3);

    run.submit_review(automator, 3);
    run.build_daily_queue(0);
    run.preview(automator);
    run.advance(Duration::minutes(10));
    run.submit_review(automator, 3);
    run.advance(Duration::days(3) + Duration::nanoseconds(777));
    run.submit_review(automator, 1);
    // A clock set back by a day reviews at the previous review instead.
    run.advance(-Duration::days(1));
    run.submit_review(automator, 3);
    run.review_log_for_card(automator);
    run.preview(automator);
    run.preview(9_999);
    run.submit_review(9_999, 3);
    run.submit_review(english, 4);
    run.upsert_srs_state(&SrsStateRow {
        algorithm: "sm2".into(),
        ..due_state(learned, instant("2026-10-31T10:00:00.000Z"))
    });
    run.submit_review(learned, 3);
    run.get_srs_state(learned);
    run.build_daily_queue(1);
    run.finish()
}

/// Streaks, freezes and the session bonus in Tokyo, where the local date
/// runs nine hours ahead of the UTC one.
fn gamification_scenario() -> Value {
    let tokyo = 9 * 3600;
    let mut run = Runner::new(
        "streaks and XP in Tokyo",
        local(offset(tokyo), "2026-11-01 07:00:00.000"),
        tokyo,
    );
    for (index, due) in [
        "2026-11-01T00:00:00.000Z",
        "2026-11-11T14:59:59.999Z",
        "2026-11-11T15:00:00.000Z",
    ]
    .iter()
    .enumerate()
    {
        let id = run.insert_capture(
            &plain(&format!("card {index}"), "Karte", "en", "de"),
            CaptureOrigin::Capture,
        );
        run.upsert_srs_state(&due_state(id, instant(due)));
    }
    run.on_session_completed();
    for day in 1..=7 {
        let text = format!("2026-11-{day:02}");
        run.set_local(&format!("{text} 21:30:00.000"));
        run.complete_day(&text);
        run.on_session_completed();
        run.on_session_completed();
        run.award_xp(10, reason::REVIEW);
    }
    run.stats_summary();
    // Nov 8 is missed; Nov 9 spends the freeze.
    run.set_local("2026-11-09 06:00:00.000");
    run.apply_streak_freezes();
    run.stats_summary();
    run.complete_day("2026-11-09");
    run.on_session_completed();
    // Two missed days with no freeze left: the streak breaks.
    run.set_local("2026-11-12 08:30:00.000");
    run.apply_streak_freezes();
    run.stats_summary();
    run.set_state(STREAK_FREEZES_KEY, "2");
    run.apply_streak_freezes();
    run.stats_summary();
    run.insert_streak_freeze("2026-11-12");
    run.insert_streak_freeze("2026-11-12");
    run.stats_summary();
    run.award_xp(15, reason::NEW_ITEM);
    run.award_xp(XP_EXERCISE_CORRECT, reason::EXERCISE);
    run.set_local("2026-11-11 09:00:00.000");
    run.stats_summary();
    run.set_state(STREAK_FREEZES_KEY, " 7 ");
    run.stats_summary();
    run.set_state(STREAK_FREEZES_KEY, "+1");
    run.stats_summary();
    run.set_state(STREAK_FREEZES_KEY, "two");
    run.stats_summary();
    run.on_session_completed();
    run.finish()
}

/// Four weeks of completions in Berlin: a missed day first noticed by the
/// completion of the next one (today, already complete, is not the gap's
/// end), and freezes earned at 14 and 21 days but not past the cap at 28.
fn freezes_at_completion_scenario() -> Value {
    let berlin = 3600;
    let mut run = Runner::new(
        "freezes spent at completion and capped",
        local(offset(berlin), "2026-11-01 08:00:00.000"),
        berlin,
    );
    for day in (1..=28).filter(|day| *day != 8) {
        let text = format!("2026-11-{day:02}");
        run.set_local(&format!("{text} 20:00:00.000"));
        run.complete_day(&text);
        run.on_session_completed();
    }
    run.stats_summary();
    run.finish()
}

/// The pool is exactly the captures with a state; lessons use clipboard
/// captures only; deleting a capture takes it out of both.
fn pool_and_lessons_scenario() -> Value {
    let mut run = Runner::new(
        "pool and lessons from the store",
        instant("2026-10-31T08:00:00Z"),
        0,
    );
    let mut ids = Vec::new();
    let cards = [
        (
            with_example(
                chinese("学习", "lernen", "de", "xué xí"),
                "我们学习中文。",
                "Wir lernen Chinesisch.",
            ),
            CaptureOrigin::Capture,
            true,
        ),
        (
            with_example(
                chinese("中文", "Chinesisch", "de", "  "),
                "我们学习中文。",
                "Wir lernen Chinesisch.",
            ),
            CaptureOrigin::Curriculum,
            true,
        ),
        (
            with_audio(with_example(
                chinese("我们", "wir", "de", "wǒ men"),
                "我们是朋友。",
                "Wir sind Freunde.",
            )),
            CaptureOrigin::Capture,
            true,
        ),
        (
            with_example(
                chinese("朋友", "Freund", "de", "péng you"),
                "我们是朋友。",
                "Wir sind Freunde.",
            ),
            CaptureOrigin::AnkiImport,
            false,
        ),
        (
            with_example(
                plain("house", "Haus", "en", "de"),
                "The house is big.",
                "Das Haus ist groß.",
            ),
            CaptureOrigin::Capture,
            true,
        ),
        (
            with_example(
                plain("cat", "Katze", "en", "de"),
                "The cat sleeps.",
                "Die Katze schläft.",
            ),
            CaptureOrigin::Capture,
            false,
        ),
        (
            with_example(plain("tree", "Baum", "en", "de"), "A tree.", "Ein Baum."),
            CaptureOrigin::AnkiImport,
            true,
        ),
    ];
    for (index, (card, origin, learned)) in cards.into_iter().enumerate() {
        let card = captured_at(card, &format!("2026-10-0{}T08:00:00Z", 7 - index));
        let id = run.insert_capture(&card, origin);
        if learned {
            run.upsert_srs_state(&learned_state(id));
        }
        ids.push(id);
    }
    run.load_pool();
    run.lessons_from_store(2);
    run.lessons_from_store(0);
    run.generate_exercise(ids[0], ExerciseKind::SentenceBuilder, 5);
    run.generate_exercise(ids[3], ExerciseKind::MultipleChoice, 5);
    run.generate_exercise(ids[4], ExerciseKind::MultipleChoice, 5);
    run.delete_capture(ids[2]);
    run.delete_capture(ids[2]);
    run.load_pool();
    run.generate_exercise(ids[2], ExerciseKind::ListeningChoice, 5);
    run.generate_exercise(ids[0], ExerciseKind::ListeningSentence, 5);
    run.lessons_from_store(2);
    run.upsert_srs_state(&learned_state(ids[3]));
    run.load_pool();
    run.finish()
}

/// `ensure_curriculum_seeded` writes once per fingerprint.
fn seeding_scenario() -> Value {
    let mut run = Runner::new("curriculum seeding", instant("2026-10-31T08:00:00Z"), 0);
    let items = scrambled_curriculum();
    run.ensure_curriculum_seeded(&items);
    run.ensure_curriculum_seeded(&items);
    let mut reversed = items.clone();
    reversed.reverse();
    run.ensure_curriculum_seeded(&reversed);
    let mut edited = items.clone();
    edited[0].meaning_en = Some("hello there".into());
    run.ensure_curriculum_seeded(&edited);
    run.ensure_curriculum_seeded(&edited[..3]);
    run.replace_curriculum(&[curriculum_item("我", 1), curriculum_item("你", 1)]);
    run.replace_curriculum(&[curriculum_item("我", 1), curriculum_item("我", 2)]);
    run.ensure_curriculum_seeded(&edited[..3]);
    run.get_state(vocab_core::store::CURRICULUM_FINGERPRINT_KEY);
    run.finish()
}

// ---------------------------------------------------------------------------
// The Learn commands, composed from the core as vocab-learn composes them

/// vocab-learn's commands (apps/learn/src-tauri: `session`, `progress` and
/// `commands::*`), replayed here against the real core because vocab-core
/// cannot depend on the app crate. The web adapter must compose the core the
/// same way, so these are its golden vectors one level up: the queue offers
/// what is left of today's quota, a rating is one transaction that counts a
/// first review as a new item (`XP_NEW_ITEM`) and any other as a review
/// (`XP_REVIEW`), the write that completes a session runs
/// `on_session_completed`, and failures read as vocab-learn's sentences.
mod learn {
    use super::*;

    /// Cards per lesson (`progress::CARDS_PER_LESSON`).
    pub const CARDS_PER_LESSON: usize = 5;

    /// `fsrs_state` of a card that has never been reviewed.
    const NEVER_REVIEWED: &str = "new";

    /// vocab-learn's `Error`, as far as these commands reach it.
    #[derive(Debug)]
    pub enum LearnError {
        Store(StoreError),
        NoSuchCard(i64),
        NotScheduled(i64),
        NotLearned(i64),
        NotInCurriculum(String),
        NoSuchLesson(String),
        Quiz(QuizError),
    }

    impl Display for LearnError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                LearnError::Store(err) => write!(f, "The vocabulary database failed ({err})."),
                LearnError::NoSuchCard(id) => write!(f, "There is no card {id}."),
                LearnError::NotScheduled(id) => {
                    write!(f, "Card {id} is not scheduled for review.")
                }
                LearnError::NotLearned(id) => write!(f, "Card {id} is not learned vocabulary."),
                LearnError::NotInCurriculum(hanzi) => {
                    write!(f, "\"{hanzi}\" is not in the HSK curriculum.")
                }
                LearnError::NoSuchLesson(id) => write!(f, "There is no lesson \"{id}\"."),
                LearnError::Quiz(err) => write!(f, "No exercise: {err}."),
            }
        }
    }

    impl From<StoreError> for LearnError {
        fn from(err: StoreError) -> Self {
            LearnError::Store(err)
        }
    }

    impl From<QuizError> for LearnError {
        fn from(err: QuizError) -> Self {
            LearnError::Quiz(err)
        }
    }

    pub type LearnResult<T> = Result<T, LearnError>;

    /// `session::daily_queue`: due cards, then what is left of the quota.
    pub fn daily_queue(store: &Store, clock: &dyn Clock) -> StoreResult<Vec<QueueItem>> {
        let today = clock.today();
        let (_, end_of_today) = local_day_bounds(today, clock.local_offset());
        let due = store.list_srs_due_before(end_of_today)?;
        let new_quota = remaining_new_quota(store.get_daily_session(today)?.as_ref(), &due);
        let curriculum = store.curriculum_in_order()?;
        let learned_hanzi = store.learned_chinese_fronts()?;
        Ok(build_daily_queue_from(
            &due,
            &curriculum,
            &learned_hanzi,
            new_quota,
        ))
    }

    /// `session::remaining_new_quota`: the quota, minus the new items done,
    /// minus the cards still waiting for their first review.
    fn remaining_new_quota(session: Option<&DailySessionRow>, due: &[SrsStateRow]) -> u32 {
        let Some(session) = session else {
            return 0;
        };
        let waiting = due
            .iter()
            .filter(|state| state.fsrs_state == NEVER_REVIEWED)
            .count();
        session
            .requested_new_count
            .saturating_sub(session.new_items_completed)
            .saturating_sub(u32::try_from(waiting).unwrap_or(u32::MAX))
    }

    /// `session::track_completion`.
    pub fn track_completion(
        store: &Store,
        clock: &dyn Clock,
        write: impl FnOnce(&Store) -> StoreResult<DailySessionRow>,
    ) -> StoreResult<(DailySessionRow, Option<StreakSummary>)> {
        store.with_transaction(|store| {
            let before = store.get_daily_session(clock.today())?;
            let session = write(store)?;
            let streak = if just_completed(before.as_ref(), &session) {
                Some(on_session_completed(store, clock)?)
            } else {
                None
            };
            Ok((session, streak))
        })
    }

    fn just_completed(before: Option<&DailySessionRow>, after: &DailySessionRow) -> bool {
        let was_completed = before.is_some_and(|before| {
            before.session_date == after.session_date && before.completed_at.is_some()
        });
        after.completed_at.is_some() && !was_completed
    }

    /// `commands::review::submit_review_rating` and `session::rate`, the
    /// result as its `ReviewResult` JSON.
    pub fn submit_review_rating(
        store: &Store,
        scheduler: &SrsScheduler,
        card_id: i64,
        given: ReviewRating,
        clock: &dyn Clock,
    ) -> LearnResult<Value> {
        if store.get_srs_state(card_id)?.is_none() {
            return Err(LearnError::NotScheduled(card_id));
        }
        Ok(store.with_transaction(|store| {
            let today = clock.today();
            let xp_before = store.xp_for_date(today)?;
            let outcome = submit_review(store, scheduler, card_id, given, clock)?;
            let (progress, xp, why) = if outcome.log.state_before == NEVER_REVIEWED {
                (SessionProgress::NewItem, XP_NEW_ITEM, reason::NEW_ITEM)
            } else {
                (SessionProgress::Review, XP_REVIEW, reason::REVIEW)
            };
            award_xp(store, xp, why, clock)?;
            let (session, completed) = track_completion(store, clock, |store| {
                record_session_progress(store, progress, clock)
            })?;
            let session_completed = completed.is_some();
            let streak = match completed {
                Some(streak) => streak,
                None => stats_summary(store, clock)?.streak,
            };
            Ok(json!({
                "next": outcome.next,
                "xp_awarded": store.xp_for_date(today)? - xp_before,
                "session": session,
                "session_completed": session_completed,
                "streak": streak,
            }))
        })?)
    }

    /// `commands::daily::start_new_item`: the word trimmed, a missing one
    /// reported as not in the curriculum.
    pub fn start_new_item(
        store: &Store,
        scheduler: &SrsScheduler,
        hanzi: &str,
        meaning_lang: &str,
        clock: &dyn Clock,
    ) -> LearnResult<i64> {
        let hanzi = hanzi.trim();
        introduce_new_item(store, scheduler, hanzi, meaning_lang, clock).map_err(|err| match err {
            StoreError::NotFound(_) => LearnError::NotInCurriculum(hanzi.to_owned()),
            other => other.into(),
        })
    }

    /// `progress::record_exercise`: only a learned card's results count.
    pub fn record_exercise(
        store: &Store,
        card_id: i64,
        correct: bool,
        clock: &dyn Clock,
    ) -> LearnResult<i64> {
        if store.get_srs_state(card_id)?.is_none() {
            return Err(LearnError::NotLearned(card_id));
        }
        if !correct {
            return Ok(0);
        }
        award_xp(store, XP_EXERCISE_CORRECT, reason::EXERCISE, clock)?;
        Ok(XP_EXERCISE_CORRECT)
    }

    /// `progress::complete_lesson`: once per lesson per local day.
    pub fn complete_lesson(store: &Store, lesson_id: &str, clock: &dyn Clock) -> LearnResult<i64> {
        let exists = lessons_from_store(store, CARDS_PER_LESSON)?
            .iter()
            .any(|lesson| lesson.id == lesson_id);
        if !exists {
            return Err(LearnError::NoSuchLesson(lesson_id.to_owned()));
        }
        let key = format!("learn.lesson_completed.{lesson_id}");
        let today = format_date(clock.today());
        Ok(store.with_transaction(|store| {
            if store.get_state(&key)?.as_deref() == Some(today.as_str()) {
                return Ok(0);
            }
            award_xp(store, XP_LESSON_COMPLETE, reason::LESSON, clock)?;
            store.set_state(&key, &today)?;
            Ok(XP_LESSON_COMPLETE)
        })?)
    }
}

use learn::{LearnError, LearnResult, CARDS_PER_LESSON};

/// Runs Learn's commands against one store and records each with its
/// arguments and result, for the web adapter to replay.
struct Learn {
    run: Runner,
    meaning_lang: &'static str,
}

impl Learn {
    fn new(
        name: &str,
        start: DateTime<Utc>,
        offset_seconds: i32,
        meaning_lang: &'static str,
    ) -> Self {
        Self {
            run: Runner::new(name, start, offset_seconds),
            meaning_lang,
        }
    }

    fn finish(self, curriculum: &[CurriculumItem]) -> Value {
        let meaning_lang = self.meaning_lang;
        let mut scenario = self.run.finish();
        scenario["meaning_lang"] = json!(meaning_lang);
        scenario["curriculum"] = to_json(curriculum);
        scenario
    }

    fn command<T: Serialize>(&mut self, command: &str, args: Value, result: LearnResult<T>) {
        self.run
            .record(json!({ "command": command, "args": args }), outcome(result));
    }

    fn store(&self) -> &Store {
        &self.run.store
    }

    fn get_daily_prompt_state(&mut self) {
        let result = daily_prompt_state(self.store(), &self.run.clock);
        self.command("getDailyPromptState", json!([]), result.map_err(Into::into));
    }

    fn submit_daily_quota(&mut self, new_count: u32) {
        let clock = &self.run.clock;
        let result = learn::track_completion(self.store(), clock, |store| {
            submit_daily_quota(store, new_count, clock)
        })
        .map(|(session, _)| session);
        self.command(
            "submitDailyQuota",
            json!([new_count]),
            result.map_err(Into::into),
        );
    }

    fn dismiss_daily_prompt(&mut self) {
        let clock = &self.run.clock;
        let result = learn::track_completion(self.store(), clock, |store| {
            dismiss_daily_prompt(store, clock)
        })
        .map(|(session, _)| session);
        self.command("dismissDailyPrompt", json!([]), result.map_err(Into::into));
    }

    fn reopen_daily_prompt(&mut self) {
        let result = reopen_daily_prompt(self.store(), &self.run.clock);
        self.command("reopenDailyPrompt", json!([]), result.map_err(Into::into));
    }

    fn get_daily_queue(&mut self) {
        let result = learn::daily_queue(self.store(), &self.run.clock);
        self.command("getDailyQueue", json!([]), result.map_err(Into::into));
    }

    fn start_new_item(&mut self, hanzi: &str) -> Option<i64> {
        let result = learn::start_new_item(
            self.store(),
            &self.run.scheduler,
            hanzi,
            self.meaning_lang,
            &self.run.clock,
        );
        let id = result.as_ref().ok().copied();
        self.command("startNewItem", json!([hanzi]), result);
        id
    }

    fn capture(&self, card_id: i64) -> LearnResult<StoredCapture> {
        self.store()
            .get_capture(card_id)?
            .ok_or(LearnError::NoSuchCard(card_id))
    }

    fn get_card(&mut self, card_id: i64) {
        let result = self.capture(card_id);
        self.command("getCard", json!([card_id]), result);
    }

    /// The stored recording; the web build has no TTS to make a missing one.
    fn get_card_audio(&mut self, card_id: i64) {
        let result = self.capture(card_id).map(|capture| {
            capture
                .card
                .audio_front
                .filter(|audio| !audio.is_empty())
                .map(|audio| STANDARD.encode(audio))
        });
        self.command("getCardAudio", json!([card_id]), result);
    }

    fn preview_ratings(&mut self, card_id: i64) {
        let state = self
            .store()
            .get_srs_state(card_id)
            .map_err(LearnError::from)
            .and_then(|state| state.ok_or(LearnError::NotScheduled(card_id)));
        let result = state.map(|state| self.run.scheduler.preview(&state, self.run.clock.now()));
        self.command("previewRatings", json!([card_id]), result);
    }

    fn submit_review_rating(&mut self, card_id: i64, given: u8) {
        let result = learn::submit_review_rating(
            self.store(),
            &self.run.scheduler,
            card_id,
            rating(given),
            &self.run.clock,
        );
        self.command("submitReviewRating", json!([card_id, given]), result);
    }

    fn list_learned_cards(&mut self) {
        let result = LearnedVocabularyPool::load(self.store()).map(|pool| pool.entries().to_vec());
        self.command("listLearnedCards", json!([]), result.map_err(Into::into));
    }

    fn next_exercise(&mut self, card_id: i64, kind: ExerciseKind, seed: u32) -> Option<Exercise> {
        let result = LearnedVocabularyPool::load(self.store())
            .map_err(LearnError::from)
            .and_then(|pool| Ok(generate_exercise(&pool, card_id, kind, seed)?));
        let exercise = result.as_ref().ok().cloned();
        self.command("nextExercise", json!([card_id, kind, seed]), result);
        exercise
    }

    fn check_exercise(&mut self, exercise: &Exercise, answer: ExerciseAnswer) {
        let correct = check_answer(exercise, &answer);
        self.command("checkExercise", json!([exercise, answer]), Ok(correct));
    }

    fn record_exercise_result(&mut self, card_id: i64, kind: ExerciseKind, correct: bool) {
        let result = learn::record_exercise(self.store(), card_id, correct, &self.run.clock);
        self.command(
            "recordExerciseResult",
            json!([card_id, kind, correct]),
            result,
        );
    }

    fn get_stats(&mut self) {
        let result = stats_summary(self.store(), &self.run.clock);
        self.command("getStats", json!([]), result.map_err(Into::into));
    }

    fn list_lessons(&mut self) {
        let result = lessons_from_store(self.store(), CARDS_PER_LESSON);
        self.command("listLessons", json!([]), result.map_err(Into::into));
    }

    fn complete_lesson(&mut self, lesson_id: &str) {
        let result = learn::complete_lesson(self.store(), lesson_id, &self.run.clock);
        self.command("completeLesson", json!([lesson_id]), result);
    }
}

fn adapter_curriculum() -> Vec<CurriculumItem> {
    let mut items = vec![
        curriculum_item("我", 0),
        curriculum_item("你", 1),
        curriculum_item("好", 2),
        curriculum_item("是", 3),
        curriculum_item("学习", 4),
        curriculum_item("中文", 5),
        curriculum_item("朋友", 6),
        curriculum_item("老师", 7),
    ];
    for item in &mut items {
        item.meaning_de = None;
        item.example_sentence_source = match item.hanzi.as_str() {
            "我" => Some("我是老师。".into()),
            "你" => Some("你好！".into()),
            "学习" => Some("我学习中文。".into()),
            _ => None,
        };
        item.example_sentence_target = item
            .example_sentence_source
            .as_ref()
            .map(|_| format!("Sentence with {}.", item.hanzi));
    }
    items[4].meaning_de = Some("lernen".into());
    items
}

fn adapter_scenarios() -> Value {
    let curriculum = adapter_curriculum();
    let berlin = 3600;
    let mut learn = Learn::new(
        "four days of Learn at UTC+1",
        local(offset(berlin), "2026-10-31 07:30:00.000"),
        berlin,
        "de",
    );
    learn.run.replace_curriculum(&curriculum);
    // Captured in Automator before Learn ever ran: one curriculum word that
    // was never scheduled, and clipboard captures with sentences for lessons.
    let automator = learn.run.insert_capture(
        &with_audio(captured_at(
            chinese("好", "gut", "de", "hǎo"),
            "2026-10-20T10:00:00Z",
        )),
        CaptureOrigin::Capture,
    );
    let mut clipboard = Vec::new();
    for (minutes, front, sentence, translation) in [
        (
            0,
            "serendipity",
            "It was pure serendipity.",
            "Es war reiner Zufall.",
        ),
        (1, "house", "The house is big.", "Das Haus ist groß."),
        (2, "cat", "Concatenate the cat.", "Verkette die Katze."),
    ] {
        let card = captured_at(
            with_example(plain(front, "Bedeutung", "en", "de"), sentence, translation),
            &(instant("2026-10-21T10:00:00Z") + Duration::minutes(minutes)).to_rfc3339(),
        );
        clipboard.push(learn.run.insert_capture(&card, CaptureOrigin::Capture));
    }

    // Day 1: three new words.
    learn.get_daily_prompt_state();
    learn.get_daily_queue();
    learn.submit_daily_quota(3);
    learn.get_daily_prompt_state();
    learn.get_daily_queue();
    let first = learn.start_new_item("我");
    let first = first.expect("我 is in the curriculum");
    learn.start_new_item("我");
    // Introduced, not rated yet: it moves to the reviews, the quota shrinks.
    learn.get_daily_queue();
    learn.get_card(first);
    learn.get_card_audio(first);
    learn.preview_ratings(first);
    learn.run.advance(Duration::seconds(20));
    learn.submit_review_rating(first, 3);
    learn.get_daily_queue();
    let second = learn.start_new_item("你").expect("你 is in the curriculum");
    learn.submit_review_rating(second, 4);
    learn.get_daily_queue();
    let adopted = learn.start_new_item("好").expect("好 is in the curriculum");
    assert_eq!(adopted, automator, "the Automator capture is adopted");
    learn.get_card_audio(adopted);
    learn
        .run
        .advance(Duration::minutes(1) + Duration::nanoseconds(5));
    learn.submit_review_rating(adopted, 1);
    learn.get_daily_queue();
    learn.run.advance(Duration::minutes(2));
    learn.submit_review_rating(adopted, 3);
    learn.get_daily_queue();
    learn.run.advance(Duration::minutes(10));
    learn.submit_review_rating(first, 3);
    learn.get_stats();
    learn.get_daily_prompt_state();
    // Introduced, then left unrated: it waits for its first rating for days.
    let waiting = learn
        .start_new_item(" 学习 ")
        .expect("学习 is in the curriculum");
    learn.start_new_item("没有");
    learn.start_new_item("  ");
    learn.get_card(9_999);
    learn.get_card_audio(9_999);
    learn.preview_ratings(9_999);
    learn.preview_ratings(clipboard[0]);
    learn.submit_review_rating(9_999, 3);
    learn.submit_review_rating(clipboard[0], 3);

    learn.list_learned_cards();
    let mut exercises = Vec::new();
    for kind in ExerciseKind::ALL {
        for card_id in [first, second, adopted, clipboard[0], 9_999] {
            if let Some(exercise) = learn.next_exercise(card_id, kind, 17) {
                exercises.push(exercise);
            }
        }
    }
    for exercise in &exercises {
        let answer = match exercise {
            Exercise::MultipleChoice(e) => ExerciseAnswer::Choice {
                index: e.correct_index,
            },
            Exercise::ListeningChoice(e) => ExerciseAnswer::Choice {
                index: e.correct_index + 1,
            },
            Exercise::SentenceBuilder(e) => ExerciseAnswer::Order {
                tile_ids: e.solution.clone(),
            },
            Exercise::ListeningSentence(e) => ExerciseAnswer::Order {
                tile_ids: e.solution.iter().rev().copied().collect(),
            },
        };
        learn.check_exercise(exercise, answer);
    }
    learn.record_exercise_result(first, ExerciseKind::MultipleChoice, true);
    learn.record_exercise_result(first, ExerciseKind::ListeningChoice, false);
    learn.record_exercise_result(clipboard[0], ExerciseKind::MultipleChoice, true);
    learn.record_exercise_result(9_999, ExerciseKind::SentenceBuilder, false);
    learn.list_lessons();
    learn.complete_lesson("lesson-2");
    learn.complete_lesson("lesson-2");
    learn.complete_lesson("lesson-77");
    learn.get_stats();

    // Day 2: dismissed, reviews only; doing them completes the day.
    learn.run.set_local("2026-11-01 08:00:00.000");
    learn.get_daily_prompt_state();
    learn.dismiss_daily_prompt();
    learn.get_daily_queue();
    learn.get_stats();
    for card_id in [first, adopted] {
        learn.run.advance(Duration::minutes(1));
        learn.submit_review_rating(card_id, 3);
    }
    learn.get_daily_queue();
    learn.complete_lesson("lesson-2");
    learn.reopen_daily_prompt();
    learn.get_daily_prompt_state();
    learn.submit_daily_quota(1);
    learn.get_daily_queue();
    learn.get_stats();

    // Day 4: day 3 was missed and no freeze is held. Reviews only, and the
    // word left unrated since day 1 is the only thing due: its first rating
    // is a new item, and it completes the day.
    learn.run.set_local("2026-11-03 23:30:00.000");
    learn.get_stats();
    learn.get_daily_prompt_state();
    learn.submit_daily_quota(0);
    learn.get_daily_queue();
    learn.submit_review_rating(waiting, 3);
    learn.get_daily_queue();
    learn.run.set_local("2026-11-04 00:10:00.000");
    learn.get_daily_queue();
    learn.submit_daily_quota(2);
    learn.get_daily_queue();
    if let Some(id) = learn.start_new_item("中文") {
        learn.get_daily_queue();
        learn.get_card(id);
        learn.submit_review_rating(id, 2);
    }
    learn.get_daily_queue();
    learn.get_stats();

    vec![learn.finish(&curriculum)].into()
}

// ---------------------------------------------------------------------------
// Prettier's JSON layout

/// Writes JSON the way Prettier formats it with the repository's settings
/// (`printWidth` 100, two-space indent), for the shapes this file contains.
/// Prettier keeps an object expanded when its source has a line break after
/// `{`, so every non-empty object is written expanded; arrays are what
/// Prettier decides: one line when that fits, else one element per line, or,
/// for arrays of numbers, as many numbers per line as fit (`fill`). Keys are
/// sorted, so the order never depends on serde_json's `preserve_order`.
mod layout {
    use serde_json::Value;

    const PRINT_WIDTH: usize = 100;

    pub fn to_json(value: &Value) -> String {
        let mut out = String::new();
        write(&mut out, value, 0, 0, false);
        out.push('\n');
        out
    }

    /// `value` at `indent`, its first character at `column`; `comma` when a
    /// comma follows it on the same line.
    fn write(out: &mut String, value: &Value, indent: usize, column: usize, comma: bool) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                let mut entries: Vec<(&String, &Value)> = map.iter().collect();
                entries.sort_by(|a, b| a.0.cmp(b.0));
                out.push_str("{\n");
                for (index, (key, child)) in entries.iter().enumerate() {
                    let last = index + 1 == entries.len();
                    let key = scalar(&Value::String((*key).clone()));
                    push_indent(out, indent + 2);
                    out.push_str(&key);
                    out.push_str(": ");
                    write(out, child, indent + 2, indent + 2 + width(&key) + 2, !last);
                    if !last {
                        out.push(',');
                    }
                    out.push('\n');
                }
                push_indent(out, indent);
                out.push('}');
            }
            Value::Array(items) if !items.is_empty() => {
                let fits = flat(value)
                    .filter(|_| !breaks_by_shape(items))
                    .filter(|text| column + width(text) + usize::from(comma) <= PRINT_WIDTH);
                if let Some(text) = fits {
                    out.push_str(&text);
                } else if items.len() > 1 && items.iter().all(Value::is_number) {
                    fill(out, items, indent);
                } else {
                    out.push_str("[\n");
                    for (index, item) in items.iter().enumerate() {
                        let last = index + 1 == items.len();
                        push_indent(out, indent + 2);
                        write(out, item, indent + 2, indent + 2, !last);
                        if !last {
                            out.push(',');
                        }
                        out.push('\n');
                    }
                    push_indent(out, indent);
                    out.push(']');
                }
            }
            other => out.push_str(&flat(other).unwrap_or_default()),
        }
    }

    /// Prettier's `fill`: each number goes on the current line when it and
    /// the next one fit there.
    fn fill(out: &mut String, items: &[Value], indent: usize) {
        let texts: Vec<String> = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let comma = if index + 1 < items.len() { "," } else { "" };
                format!("{}{comma}", scalar(item))
            })
            .collect();
        out.push_str("[\n");
        push_indent(out, indent + 2);
        let mut position = indent + 2;
        for (index, text) in texts.iter().enumerate() {
            let start = position;
            out.push_str(text);
            position += width(text);
            let Some(next) = texts.get(index + 1) else {
                break;
            };
            if width(text) + 1 + width(next) <= PRINT_WIDTH.saturating_sub(start) {
                out.push(' ');
                position += 1;
            } else {
                out.push('\n');
                push_indent(out, indent + 2);
                position = indent + 2;
            }
        }
        out.push('\n');
        push_indent(out, indent);
        out.push(']');
    }

    /// Prettier always breaks an array of two or more arrays that each hold
    /// more than one element.
    fn breaks_by_shape(items: &[Value]) -> bool {
        items.len() > 1
            && items
                .iter()
                .all(|item| item.as_array().is_some_and(|inner| inner.len() > 1))
    }

    /// The one-line form; `None` for anything holding a non-empty object,
    /// which is always expanded.
    fn flat(value: &Value) -> Option<String> {
        match value {
            Value::Object(map) => map.is_empty().then(|| "{}".to_owned()),
            Value::Array(items) => {
                let parts: Option<Vec<String>> = items.iter().map(flat).collect();
                Some(format!("[{}]", parts?.join(", ")))
            }
            other => Some(scalar(other)),
        }
    }

    fn scalar(value: &Value) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }

    fn push_indent(out: &mut String, indent: usize) {
        out.extend(std::iter::repeat_n(' ', indent));
    }

    /// Prettier's `getStringWidth`: East Asian wide and fullwidth characters
    /// take two columns, control characters and combining diacritics none.
    pub(super) fn width(text: &str) -> usize {
        text.chars()
            .map(|c| match u32::from(c) {
                0x00..=0x1F | 0x7F..=0x9F | 0x300..=0x36F => 0,
                0x1100..=0x115F
                | 0x2E80..=0x303E
                | 0x3041..=0x33FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xA000..=0xA4CF
                | 0xAC00..=0xD7A3
                | 0xF900..=0xFAFF
                | 0xFE30..=0xFE4F
                | 0xFF00..=0xFF60
                | 0xFFE0..=0xFFE6
                | 0x1F300..=0x1F64F
                | 0x1F900..=0x1F9FF
                | 0x20000..=0x3FFFD => 2,
                _ => 1,
            })
            .sum()
    }
}

// ---------------------------------------------------------------------------
// `--check`

mod compare {
    use serde_json::Value;

    /// Relative tolerance for floats: the last bits of `exp` and `powf`
    /// differ between platform maths libraries.
    const FLOAT_TOLERANCE: f64 = 1e-12;

    /// The path of the first place where `actual` differs from `expected`.
    pub fn first_difference(expected: &Value, actual: &Value) -> Option<String> {
        walk(expected, actual, "$")
    }

    fn walk(expected: &Value, actual: &Value, path: &str) -> Option<String> {
        match (expected, actual) {
            (Value::Object(a), Value::Object(b)) => {
                let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
                keys.sort();
                keys.dedup();
                keys.into_iter().find_map(|key| {
                    let path = format!("{path}.{key}");
                    match (a.get(key), b.get(key)) {
                        (Some(x), Some(y)) => walk(x, y, &path),
                        _ => Some(path),
                    }
                })
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return Some(format!("{path} (length {} vs {})", a.len(), b.len()));
                }
                a.iter()
                    .zip(b)
                    .enumerate()
                    .find_map(|(index, (x, y))| walk(x, y, &format!("{path}[{index}]")))
            }
            (Value::Number(a), Value::Number(b)) if a != b => {
                let close = a.is_f64()
                    && b.is_f64()
                    && match (a.as_f64(), b.as_f64()) {
                        (Some(x), Some(y)) => {
                            (x - y).abs() <= FLOAT_TOLERANCE * x.abs().max(y.abs()).max(1.0)
                        }
                        _ => false,
                    };
                (!close).then(|| path.to_owned())
            }
            _ => (expected != actual).then(|| path.to_owned()),
        }
    }
}
