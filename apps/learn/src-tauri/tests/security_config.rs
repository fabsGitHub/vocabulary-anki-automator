//! Learn's webview security configuration (docs/architecture.md § 10): the
//! Content Security Policy in `tauri.conf.json` and the main window's
//! capability. No other test reads these files, so a widened permission or a
//! weakened policy would otherwise only show up in a review.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, INVOKE_KEY},
    webview::InvokeRequest,
    Manager, WebviewWindowBuilder,
};

fn crate_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// A directive-map policy from `tauri.conf.json` as directive -> sources.
fn policy(value: &Value) -> BTreeMap<String, Vec<String>> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("expected a directive map, got {value}"))
        .iter()
        .map(|(name, sources)| {
            let sources = sources
                .as_str()
                .unwrap_or_else(|| panic!("{name}: expected a source string"))
                .split_whitespace()
                .map(str::to_string)
                .collect();
            (name.clone(), sources)
        })
        .collect()
}

/// What the built desktop frontend (`npm run learn:build`, apps/learn/dist/)
/// uses: its own module script and stylesheet, Vue's style bindings (the
/// progress bar), base64 MP3s played through
/// `new Audio("data:audio/mpeg;base64,...")` and Tauri's IPC
/// (`ipc://localhost`, `http://ipc.localhost` on Windows and Android).
fn expected(connect_src: &str) -> BTreeMap<String, Vec<String>> {
    [
        ("default-src", "'self'"),
        ("script-src", "'self'"),
        ("style-src", "'self' 'unsafe-inline'"),
        ("img-src", "'self'"),
        ("media-src", "data:"),
        ("connect-src", connect_src),
        ("object-src", "'none'"),
        ("base-uri", "'none'"),
        ("form-action", "'none'"),
        ("frame-src", "'none'"),
        ("frame-ancestors", "'none'"),
    ]
    .iter()
    .map(|(name, sources)| {
        (
            name.to_string(),
            sources.split_whitespace().map(str::to_string).collect(),
        )
    })
    .collect()
}

#[test]
fn the_policies_allow_exactly_what_the_frontend_uses() {
    let config = read_json(&crate_file("tauri.conf.json"));
    let security = &config["app"]["security"];
    assert_eq!(
        policy(&security["csp"]),
        expected("ipc: http://ipc.localhost")
    );
    assert_eq!(config["build"]["devUrl"], "http://localhost:1430");
    assert_eq!(
        policy(&security["devCsp"]),
        expected("ipc: http://ipc.localhost ws://localhost:1430"),
        "dev adds only the Vite HMR socket"
    );
    assert!(security
        .get("dangerousDisableAssetCspModification")
        .is_none());
    assert!(security.get("assetProtocol").is_none(), "no asset protocol");
    assert_ne!(config["app"]["withGlobalTauri"], Value::Bool(true));
}

/// The frontend only invokes Learn's own commands, which Tauri allows for the
/// app's local origin without any permission; `core:default` would add tray
/// icons, menus, event emitting and `image|from_path` (inert only because
/// this build lacks the `image-png` and `image-ico` features).
#[test]
fn the_main_window_is_granted_no_core_or_plugin_command() {
    let dir = crate_file("capabilities");
    let files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let capability = read_json(&files[0]);
    assert_eq!(capability["identifier"], "default");
    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert_eq!(capability["permissions"], serde_json::json!([]));
    assert!(capability.get("remote").is_none());
}

/// The same through the IPC layer, with the ACL the shipped app enforces
/// (`generate_context!` reads the real config and capabilities).
#[test]
fn the_ipc_refuses_every_core_command_to_the_main_window() {
    let app = vocab_learn_lib::configure(mock_builder())
        .build(tauri::generate_context!())
        .expect("mock app with the real context");
    let main = app.get_webview_window("main").unwrap_or_else(|| {
        WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap()
    });
    let url = if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };
    let refused = [
        ("plugin:image|from_path", json!({ "path": "/etc/hosts" })),
        ("plugin:tray|new", json!({ "options": {} })),
        ("plugin:menu|new", json!({ "kind": "Menu", "options": {} })),
        ("plugin:event|emit", json!({ "event": "x", "payload": {} })),
        (
            "plugin:event|listen",
            json!({ "event": "x", "target": { "kind": "Any" }, "handler": 1 }),
        ),
        ("plugin:window|hide", json!({ "label": "main" })),
        (
            "plugin:webview|create_webview_window",
            json!({ "options": {} }),
        ),
    ];
    for (command, args) in refused {
        let result = get_ipc_response(
            &main,
            InvokeRequest {
                cmd: command.into(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: url.parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        assert!(
            matches!(&result, Err(err) if err.to_string().contains("not allowed")),
            "{command}: {result:?}"
        );
    }
}
