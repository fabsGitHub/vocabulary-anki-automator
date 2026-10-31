//! The release side of `tauri.conf.json` (Milestone N).
//!
//! Tauri reads this file literally: it expands no `{{...}}` templates and no
//! environment variables in it. The signing settings that come from CI
//! secrets are read by the Tauri CLI from the environment itself
//! (`APPLE_SIGNING_IDENTITY`, `APPLE_CERTIFICATE`, `TAURI_SIGNING_PRIVATE_KEY`,
//! ...; docs/signing-key-custody.md). A template left in
//! `bundle.macOS.signingIdentity` was therefore the identity `codesign` was
//! asked for whenever `APPLE_SIGNING_IDENTITY` was unset, and every macOS
//! bundle built outside CI failed to sign.

use serde_json::Value;

const CONFIG: &str = include_str!("../tauri.conf.json");

fn config() -> Value {
    serde_json::from_str(CONFIG).expect("tauri.conf.json is JSON")
}

/// Every string in `value` with its JSON path.
fn strings(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::String(text) => out.push((path.to_string(), text.clone())),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                strings(item, &format!("{path}[{index}]"), out);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                strings(item, &format!("{path}.{key}"), out);
            }
        }
        _ => {}
    }
}

#[test]
fn the_config_holds_no_templates_tauri_would_take_literally() {
    let mut found = Vec::new();
    strings(&config(), "", &mut found);
    let templates: Vec<_> = found
        .into_iter()
        .filter(|(_, text)| text.contains("{{") || text.contains("${"))
        .collect();
    assert!(templates.is_empty(), "unexpanded templates: {templates:?}");
}

#[test]
fn the_macos_signing_identity_comes_from_the_environment() {
    let config = config();
    let macos = &config["bundle"]["macOS"];
    assert!(macos.is_object(), "bundle.macOS keeps the entitlements");
    assert_eq!(
        macos.get("signingIdentity"),
        None,
        "the Tauri CLI reads APPLE_SIGNING_IDENTITY; a value here makes every \
         local macOS bundle need that certificate"
    );
}
