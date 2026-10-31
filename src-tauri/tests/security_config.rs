//! The webview's security configuration (docs/architecture.md § 8.3): the
//! Content Security Policy in `tauri.conf.json`, the capabilities of each
//! window and the macOS hardened-runtime entitlements. These are plain files
//! that no other test reads, so a widened permission or a weakened policy
//! would otherwise only show up in a review.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde_json::{json, Value};
use tauri::{
    ipc::{CallbackFn, InvokeBody},
    test::{get_ipc_response, mock_builder, MockRuntime, INVOKE_KEY},
    webview::InvokeRequest,
    Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use vocab_automator_lib::{capture::PREVIEW_LABEL, MAIN_LABEL};

fn crate_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn config() -> Value {
    read_json(&crate_file("tauri.conf.json"))
}

/// A policy from `tauri.conf.json` (a string or a directive map, as Tauri
/// accepts either) as directive -> sources.
fn policy(value: &Value) -> BTreeMap<String, Vec<String>> {
    let split = |text: &str| text.split_whitespace().map(str::to_string).collect();
    match value {
        Value::String(text) => text
            .split(';')
            .filter_map(|directive| {
                let mut words = directive.split_whitespace();
                let name = words.next()?.to_string();
                Some((name, words.map(str::to_string).collect()))
            })
            .collect(),
        Value::Object(map) => map
            .iter()
            .map(|(name, sources)| {
                let sources = match sources {
                    Value::String(text) => split(text),
                    Value::Array(list) => list
                        .iter()
                        .map(|source| source.as_str().expect("a source string").to_string())
                        .collect(),
                    other => panic!("{name}: unexpected sources {other}"),
                };
                (name.clone(), sources)
            })
            .collect(),
        other => panic!("expected a Content Security Policy, got {other}"),
    }
}

/// What the built frontend (`npm run build`, dist/) actually uses: its own
/// module script and stylesheet, Vue's style bindings, base64 MP3s played
/// through `new Audio("data:audio/mpeg;base64,...")`, and Tauri's IPC, which
/// is `ipc://localhost` on macOS and Linux and `http://ipc.localhost` on
/// Windows. Nothing remote, no eval, no frames, no plugins, no forms that
/// post anywhere.
fn expected(connect_src: &[&str]) -> BTreeMap<String, Vec<String>> {
    let entries: [(&str, &[&str]); 11] = [
        ("default-src", &["'self'"]),
        ("script-src", &["'self'"]),
        ("style-src", &["'self'", "'unsafe-inline'"]),
        ("img-src", &["'self'"]),
        ("media-src", &["data:"]),
        ("connect-src", connect_src),
        ("object-src", &["'none'"]),
        ("base-uri", &["'none'"]),
        ("form-action", &["'none'"]),
        ("frame-src", &["'none'"]),
        ("frame-ancestors", &["'none'"]),
    ];
    entries
        .iter()
        .map(|(name, sources)| {
            (
                name.to_string(),
                sources.iter().map(|s| s.to_string()).collect(),
            )
        })
        .collect()
}

/// Rules any policy of this app must keep, whatever else changes.
fn assert_strict(name: &str, csp: &BTreeMap<String, Vec<String>>, allowed_hosts: &[&str]) {
    for (directive, sources) in csp {
        for source in sources {
            assert_ne!(source, "*", "{name} {directive}: wildcard source");
            assert_ne!(source, "'unsafe-eval'", "{name} {directive}: eval");
            assert_ne!(source, "'unsafe-hashes'", "{name} {directive}");
            assert!(
                !matches!(
                    source.as_str(),
                    "http:" | "https:" | "ws:" | "wss:" | "blob:"
                ),
                "{name} {directive}: scheme-wide source {source}"
            );
            if source.contains("://") {
                assert!(
                    allowed_hosts.contains(&source.as_str()),
                    "{name} {directive}: remote source {source}"
                );
            }
        }
    }
    let script = &csp["script-src"];
    assert!(
        !script.iter().any(|source| source == "'unsafe-inline'"),
        "{name}: inline scripts"
    );
}

#[test]
fn the_production_policy_allows_exactly_what_the_frontend_uses() {
    let config = config();
    let security = &config["app"]["security"];
    let csp = policy(&security["csp"]);
    assert_eq!(csp, expected(&["ipc:", "http://ipc.localhost"]));
    assert_strict("csp", &csp, &["http://ipc.localhost"]);
    // Tauri must keep adding its hashes of the bundled scripts.
    assert!(security
        .get("dangerousDisableAssetCspModification")
        .is_none());
    assert!(security.get("assetProtocol").is_none(), "no asset protocol");
    assert_ne!(config["app"]["withGlobalTauri"], Value::Bool(true));
}

#[test]
fn the_dev_policy_only_adds_the_vite_hmr_socket() {
    let config = config();
    let dev_url = config["build"]["devUrl"].as_str().unwrap();
    assert_eq!(dev_url, "http://localhost:1420");
    let dev = policy(&config["app"]["security"]["devCsp"]);
    assert_eq!(
        dev,
        expected(&["ipc:", "http://ipc.localhost", "ws://localhost:1420"])
    );
    assert_strict(
        "devCsp",
        &dev,
        &["http://ipc.localhost", "ws://localhost:1420"],
    );
}

/// Every capability file as identifier -> (windows, permissions).
fn capabilities(dir: &Path) -> BTreeMap<String, (Vec<String>, BTreeSet<String>)> {
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .expect("an array")
            .iter()
            .map(|item| item.as_str().expect("a plain string entry").to_string())
            .collect()
    };
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let capability = read_json(&path);
        let identifier = capability["identifier"].as_str().unwrap().to_string();
        assert!(
            capability.get("remote").is_none(),
            "{identifier}: no remote origin may reach the IPC"
        );
        assert!(capability.get("webviews").is_none(), "{identifier}");
        let permissions: BTreeSet<String> =
            strings(&capability["permissions"]).into_iter().collect();
        found.insert(identifier, (strings(&capability["windows"]), permissions));
    }
    found
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// `core:default` alone grants creating tray icons and menus, emitting events
/// to other windows (a fake `translation-payload` for the approval window) and
/// `image|from_path` (inert only because this build lacks the `image-png` and
/// `image-ico` features); `opener:default` opens any web or mail URL and
/// reveals any file. Neither window calls any of that, so each gets exactly
/// what its code does (src/api/events.ts, src/api/window.ts,
/// src/composables/useUpdater.ts).
#[test]
fn each_window_gets_exactly_the_permissions_its_code_uses() {
    let found = capabilities(&crate_file("capabilities"));
    let expected = BTreeMap::from([
        (
            "default".to_string(),
            (
                vec![MAIN_LABEL.to_string()],
                set(&[
                    "core:event:allow-listen",
                    "core:event:allow-unlisten",
                    "core:resources:allow-close",
                    "updater:allow-check",
                    "updater:allow-download",
                    "updater:allow-install",
                ]),
            ),
        ),
        (
            "preview".to_string(),
            (
                vec![PREVIEW_LABEL.to_string()],
                set(&[
                    "core:event:allow-listen",
                    "core:event:allow-unlisten",
                    "core:window:allow-hide",
                ]),
            ),
        ),
    ]);
    assert_eq!(found, expected);
    for (identifier, (_, permissions)) in &found {
        for permission in permissions {
            assert!(
                !permission.ends_with(":default") && permission != "core:default",
                "{identifier}: permission sets such as {permission} grant more than is used"
            );
        }
    }
}

/// Both windows of a mock app built from the real `tauri.conf.json` and
/// capabilities (`generate_context!`), so IPC requests meet the very ACL the
/// shipped app enforces. The plugins are not registered: a refusal by the ACL
/// comes before any plugin is asked.
fn windows_with_the_real_acl() -> (
    tauri::App<MockRuntime>,
    WebviewWindow<MockRuntime>,
    WebviewWindow<MockRuntime>,
) {
    let app = vocab_automator_lib::configure(mock_builder())
        .build(tauri::generate_context!())
        .expect("mock app with the real context");
    let main = app.get_webview_window(MAIN_LABEL).unwrap_or_else(|| {
        WebviewWindowBuilder::new(&app, MAIN_LABEL, Default::default())
            .build()
            .unwrap()
    });
    let preview =
        WebviewWindowBuilder::new(&app, PREVIEW_LABEL, WebviewUrl::App("index.html".into()))
            .build()
            .expect("preview window");
    (app, main, preview)
}

fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, String> {
    let url = if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };
    get_ipc_response(
        window,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: url.parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap_or(Value::Null))
    .map_err(|err| err.to_string())
}

fn refused_by_acl(result: &Result<Value, String>) -> bool {
    matches!(result, Err(message) if message.contains("not allowed"))
}

#[test]
fn the_ipc_lets_each_window_make_its_own_calls_and_nothing_else() {
    let (_app, main, preview) = windows_with_the_real_acl();
    let listen = json!({ "event": "capture-error", "target": { "kind": "Any" }, "handler": 7 });

    // What the frontend calls.
    for window in [&main, &preview] {
        let result = invoke(window, "plugin:event|listen", listen.clone());
        assert!(result.is_ok(), "{}: listen {result:?}", window.label());
        let unlisten = json!({ "event": "capture-error", "eventId": result.unwrap() });
        let result = invoke(window, "plugin:event|unlisten", unlisten);
        assert!(result.is_ok(), "{}: unlisten {result:?}", window.label());
    }
    let hide = invoke(
        &preview,
        "plugin:window|hide",
        json!({ "label": PREVIEW_LABEL }),
    );
    assert!(
        !refused_by_acl(&hide),
        "the approval window hides itself: {hide:?}"
    );
    for command in ["check", "download", "install"] {
        // The updater plugin is not loaded here, so an allowed call fails
        // later, as "plugin not found", never as an ACL refusal.
        let result = invoke(&main, &format!("plugin:updater|{command}"), json!({}));
        assert!(!refused_by_acl(&result), "updater {command}: {result:?}");
    }

    // What no window may do.
    let refused: [(&WebviewWindow<MockRuntime>, &str, Value); 10] = [
        (
            &main,
            "plugin:opener|open_url",
            json!({ "url": "https://example.com" }),
        ),
        (
            &main,
            "plugin:opener|reveal_item_in_dir",
            json!({ "paths": ["/etc"] }),
        ),
        (
            &main,
            "plugin:image|from_path",
            json!({ "path": "/etc/hosts" }),
        ),
        (&main, "plugin:tray|new", json!({ "options": {} })),
        (
            &main,
            "plugin:event|emit",
            json!({ "event": "translation-payload", "payload": {} }),
        ),
        (&main, "plugin:updater|download_and_install", json!({})),
        (&main, "plugin:window|hide", json!({ "label": MAIN_LABEL })),
        (&preview, "plugin:updater|check", json!({})),
        (
            &preview,
            "plugin:window|close",
            json!({ "label": PREVIEW_LABEL }),
        ),
        (
            &preview,
            "plugin:event|emit_to",
            json!({ "target": { "kind": "Any" }, "event": "x" }),
        ),
    ];
    for (window, command, args) in refused {
        let result = invoke(window, command, args);
        assert!(
            refused_by_acl(&result),
            "{}: {command} {result:?}",
            window.label()
        );
    }
}

/// The app holds the Accessibility permission, which macOS ties to its code
/// signature; a hardened-runtime exception would let foreign code run with it.
#[test]
fn the_hardened_runtime_has_no_exceptions() {
    let config = config();
    let file = config["bundle"]["macOS"]["entitlements"].as_str().unwrap();
    let plist = fs::read_to_string(crate_file(file)).unwrap();
    let body: String = {
        // Comments may name the entitlements this file must not have.
        let mut rest = plist.as_str();
        let mut body = String::new();
        while let Some(start) = rest.find("<!--") {
            body.push_str(&rest[..start]);
            let end = rest[start..].find("-->").expect("a closed comment");
            rest = &rest[start + end + 3..];
        }
        body.push_str(rest);
        body
    };
    assert!(body.contains("<dict/>"), "{body}");
    assert!(!body.contains("<key>"), "no entitlements expected:\n{body}");
}
