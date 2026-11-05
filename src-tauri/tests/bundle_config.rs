//! What `tauri build` reads from `tauri.conf.json` when it bundles and signs
//! a release (Milestone N, `.github/workflows/release.yml`).
//!
//! Tauri's configuration has no interpolation: every value reaches the
//! bundler verbatim. The signing inputs come from the environment instead,
//! where the CLI reads them itself (`APPLE_SIGNING_IDENTITY`,
//! `APPLE_CERTIFICATE`, `TAURI_SIGNING_PRIVATE_KEY`; release.yml passes the
//! secrets by name).

use serde_json::Value;

fn tauri_conf() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json");
    let text = std::fs::read_to_string(path).expect("src-tauri/tauri.conf.json is readable");
    serde_json::from_str(&text).expect("src-tauri/tauri.conf.json is JSON")
}

/// Every string in `value` with its JSON pointer.
fn strings(value: &Value, pointer: &str, found: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) => found.push((pointer.to_owned(), text.clone())),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                strings(item, &format!("{pointer}/{index}"), found);
            }
        }
        Value::Object(fields) => {
            for (key, field) in fields {
                strings(field, &format!("{pointer}/{key}"), found);
            }
        }
        _ => {}
    }
}

#[test]
fn no_value_is_a_placeholder_for_interpolation() {
    // `"signingIdentity": "{{env.APPLE_SIGNING_IDENTITY:::}}"` made every
    // local `tauri build` on macOS fail in codesign ("no identity found"),
    // and in CI it only worked because the environment variable overrides it.
    let mut found = Vec::new();
    strings(&tauri_conf(), "", &mut found);
    let templates: Vec<_> = found
        .into_iter()
        .filter(|(_, text)| text.contains("{{") || text.contains("${"))
        .collect();
    assert!(
        templates.is_empty(),
        "tauri.conf.json is not templated; these values would be used literally: {templates:?}"
    );
}

#[test]
fn the_macos_signing_identity_is_left_to_the_environment() {
    // Without an identity a local bundle is simply unsigned; the release
    // takes it from APPLE_SIGNING_IDENTITY (or the APPLE_CERTIFICATE it
    // imports). A committed identity would break every machine without it.
    let identity = tauri_conf()
        .pointer("/bundle/macOS/signingIdentity")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(identity, Value::Null, "bundle.macOS.signingIdentity");
}
