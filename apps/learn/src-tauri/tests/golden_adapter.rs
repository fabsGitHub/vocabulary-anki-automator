//! The web build's golden `adapter` scenario, replayed through the desktop
//! app's real commands (docs/architecture.md § 10).
//!
//! `crates/vocab-core/examples/golden_vectors.rs` records that scenario by
//! composing the core the way vocab-learn does, with a copy of vocab-learn's
//! command bodies (vocab-core cannot depend on this crate), and the web
//! adapter's Vitest suite replays it. Without this test nothing would notice
//! the copy drifting from the commands themselves, and the web build could
//! then match the golden file while disagreeing with the desktop app. Here
//! every step goes through the invoke handler of `vocab_learn_lib`, with the
//! scenario's clock, curriculum and settings, and must give the recorded
//! result: the desktop app, the golden file and the web adapter agree.
//!
//! Text-to-speech always fails here: the recorded `getCardAudio` results
//! are what a platform without TTS answers (the stored recording, or none),
//! and the desktop command answers the same once TTS gives it nothing.

mod common;

use std::sync::Arc;

use chrono::{DateTime, FixedOffset, Utc};
use common::{command_for, first_difference, test_settings, Harness, Options};
use serde_json::{json, Map, Value};
use vocab_core::{time::FixedClock, CaptureOrigin, Card, CurriculumItem};
use vocab_learn_lib::settings::LearnSettings;

const GOLDEN: &str = include_str!("../../src/platform/web/__fixtures__/golden.json");

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    value
        .get(name)
        .unwrap_or_else(|| panic!("the golden step has no `{name}`: {value}"))
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).expect("a golden value of the expected type")
}

/// How the golden file records a result: `{ "ok": value }` or `{ "error": sentence }`.
fn outcome<T: serde::Serialize, E: std::fmt::Display>(result: Result<T, E>) -> Value {
    match result {
        Ok(value) => json!({ "ok": value }),
        Err(err) => json!({ "error": err.to_string() }),
    }
}

/// The harness the scenario starts in: its clock, curriculum and meaning
/// language, and a TTS that never answers.
fn harness_for(scenario: &Value) -> Harness {
    let start: DateTime<Utc> = parse(field(scenario, "start"));
    let offset_seconds: i32 = parse(field(scenario, "offset_seconds"));
    let offset = FixedOffset::east_opt(offset_seconds).expect("a valid UTC offset");
    let curriculum: Vec<CurriculumItem> = parse(field(scenario, "curriculum"));
    let mut settings: LearnSettings = test_settings();
    settings.meaning_lang = parse(field(scenario, "meaning_lang"));
    let harness = Harness::with(Options {
        curriculum: Ok(curriculum),
        settings,
        clock: Arc::new(FixedClock::new(start, offset)),
        ..Options::default()
    });
    harness.tts.fail();
    harness
}

/// Runs one step and returns what it resulted in, `None` for a clock change.
fn run_step(harness: &Harness, step: &Value) -> Option<Value> {
    if let Some(op) = step.get("op").and_then(Value::as_str) {
        let store = harness.open_store();
        return match op {
            "set_clock" => {
                harness.clock.set(parse(field(step, "now")));
                None
            }
            "replace_curriculum" => {
                let items: Vec<CurriculumItem> = parse(field(step, "items"));
                let mut store = store;
                Some(outcome(store.replace_curriculum(&items)))
            }
            "insert_capture" => {
                let card: Card = parse(field(step, "card"));
                let origin: CaptureOrigin = parse(field(step, "origin"));
                Some(outcome(store.insert_capture(&card, origin, false)))
            }
            other => panic!("the replay does not know the store operation {other}"),
        };
    }
    let method = field(step, "command").as_str().expect("a method name");
    let (command, names) = command_for(method);
    let values = field(step, "args")
        .as_array()
        .expect("positional arguments");
    assert_eq!(values.len(), names.len(), "{method} takes {names:?}");
    let args: Map<String, Value> = names
        .iter()
        .map(|name| name.to_string())
        .zip(values.iter().cloned())
        .collect();
    Some(match harness.invoke(command, Value::Object(args)) {
        Ok(value) => json!({ "ok": value }),
        Err(Value::String(sentence)) => json!({ "error": sentence }),
        Err(other) => panic!("{command} failed with a non-string error {other}"),
    })
}

#[test]
fn the_desktop_commands_give_the_golden_adapter_results() {
    let golden: Value = serde_json::from_str(GOLDEN).expect("golden.json parses");
    let scenarios = field(&golden, "adapter").as_array().expect("scenarios");
    assert!(
        !scenarios.is_empty(),
        "the golden file has adapter scenarios"
    );
    let mut commands_run = 0;
    for scenario in scenarios {
        let name = field(scenario, "name").as_str().unwrap_or("unnamed");
        let harness = harness_for(scenario);
        let steps = field(scenario, "steps").as_array().expect("steps");
        for (index, step) in steps.iter().enumerate() {
            let Some(actual) = run_step(&harness, step) else {
                continue;
            };
            let expected = field(step, "result");
            if let Some(at) = first_difference(expected, &actual, "$") {
                panic!(
                    "{name}, step {index} ({}): the desktop result differs at {at}\n\
                     expected {expected}\n     got {actual}",
                    step.get("command")
                        .or(step.get("op"))
                        .unwrap_or(&Value::Null)
                );
            }
            commands_run += usize::from(step.get("command").is_some());
        }
    }
    assert!(
        commands_run > 50,
        "only {commands_run} commands were replayed"
    );
}
