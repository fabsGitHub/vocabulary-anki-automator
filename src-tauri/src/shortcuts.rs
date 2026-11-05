//! Global hotkeys (Milestone H): the default hotkey plus one per hotkey
//! profile, each capturing into its own language pair and deck.
//!
//! Shortcuts are compared as parsed key combinations, never as strings:
//! `"Shift+CommandOrControl+C"` and `"CommandOrControl+Shift+C"` are the same
//! keys, and `CommandOrControl` means Cmd on macOS but Ctrl elsewhere, so only
//! the parsed form says whether two profiles collide on this platform.
//!
//! Registration goes through the global-shortcut plugin when it is loaded and
//! is skipped otherwise, which is what lets the `tauri::test` mock app run the
//! settings commands without touching the OS.

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcut, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

use crate::{
    capture,
    config::{AppConfig, CaptureProfile},
    state::AppState,
};

/// Parses `text` the way the global-shortcut plugin does, and refuses
/// shortcuts that would swallow ordinary typing: a letter alone, or with only
/// Shift, would stop that letter from reaching any other app. Function keys
/// may stand alone.
pub fn parse_shortcut(text: &str) -> Result<Shortcut, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("no shortcut is set.".to_string());
    }
    let shortcut: Shortcut = text.parse().map_err(|_| {
        format!(
            "\"{text}\" is not a valid shortcut; use modifiers and one key, \
             e.g. \"CommandOrControl+Shift+C\"."
        )
    })?;
    let has_modifier = shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER);
    if !has_modifier && !is_function_key(shortcut.key) {
        return Err(format!(
            "\"{text}\" needs Ctrl, Cmd or Alt; a global shortcut without one would swallow \
             normal typing."
        ));
    }
    Ok(shortcut)
}

/// The profile whose shortcut is `pressed`, if any.
pub fn profile_for_shortcut(config: &AppConfig, pressed: &Shortcut) -> Option<CaptureProfile> {
    config
        .profiles()
        .into_iter()
        .find(|profile| parse_shortcut(&profile.shortcut).is_ok_and(|s| s == *pressed))
}

/// Registers every profile's shortcut, replacing whatever this app had
/// registered before. A shortcut that does not parse, repeats an earlier one
/// or is taken by another app is skipped; the error lists them all.
pub fn register_all<R: Runtime>(app: &AppHandle<R>, config: &AppConfig) -> Result<(), String> {
    let Some(manager) = app.try_state::<GlobalShortcut<R>>() else {
        log::debug!("The global shortcut plugin is not loaded; no hotkeys registered.");
        return Ok(());
    };
    if let Err(err) = manager.unregister_all() {
        log::warn!("Could not unregister the previous hotkeys: {err}");
    }
    let mut registered: Vec<Shortcut> = Vec::new();
    let mut failures = Vec::new();
    for profile in config.profiles() {
        let shortcut = match parse_shortcut(&profile.shortcut) {
            Ok(shortcut) => shortcut,
            Err(message) => {
                failures.push(message);
                continue;
            }
        };
        if registered.contains(&shortcut) {
            failures.push(format!(
                "\"{}\" is assigned twice; only the first use is active.",
                profile.shortcut
            ));
            continue;
        }
        match manager.register(shortcut) {
            Ok(()) => {
                log::info!("Hotkey {} is active.", profile.shortcut);
                registered.push(shortcut);
            }
            Err(err) => failures.push(format!(
                "\"{}\" could not be registered: {err}",
                profile.shortcut
            )),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// The global-shortcut plugin's handler: a released hotkey starts a capture
/// with its profile.
pub fn on_shortcut<R: Runtime>(app: &AppHandle<R>, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Released {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match profile_for_shortcut(&state.config(), shortcut) {
        Some(profile) => capture::start_hotkey_capture(app, profile),
        None => log::debug!("Ignoring hotkey {shortcut}: no profile uses it any more."),
    }
}

/// F1 to F24, by their W3C code names.
fn is_function_key(key: Code) -> bool {
    key.to_string()
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HotkeyProfile;

    #[test]
    fn spelling_and_order_do_not_matter() {
        let canonical = parse_shortcut("CommandOrControl+Shift+C").unwrap();
        for spelling in [
            "Shift+CommandOrControl+C",
            "commandorcontrol+shift+KeyC",
            "  CmdOrCtrl + Shift + c ",
        ] {
            assert_eq!(parse_shortcut(spelling).unwrap(), canonical, "{spelling}");
        }
        assert_ne!(parse_shortcut("CommandOrControl+Alt+C").unwrap(), canonical);
    }

    #[test]
    fn bare_and_shift_only_letters_are_refused_but_function_keys_are_not() {
        for text in ["C", "Shift+C", "shift+KeyQ"] {
            let err = parse_shortcut(text).unwrap_err();
            assert!(err.contains("needs Ctrl, Cmd or Alt"), "{text}: {err}");
        }
        for text in ["F8", "Shift+F12", "Alt+C", "Ctrl+C", "Super+C"] {
            assert!(parse_shortcut(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn unparsable_shortcuts_are_explained() {
        assert_eq!(parse_shortcut("  ").unwrap_err(), "no shortcut is set.");
        for text in ["Ctrl+Shift", "Ctrl+C+Shift", "Ctrl++C", "Ctrl+Nope"] {
            let err = parse_shortcut(text).unwrap_err();
            assert!(err.contains("is not a valid shortcut"), "{text}: {err}");
        }
    }

    #[test]
    fn a_pressed_shortcut_finds_its_profile() {
        let mut config = AppConfig::default();
        config.hotkey_profiles.push(HotkeyProfile {
            id: "zh-de".into(),
            shortcut: "CommandOrControl+Shift+H".into(),
            source_lang: "zh-CN".into(),
            target_lang: "de".into(),
            ..HotkeyProfile::default()
        });

        let pressed = |text: &str| parse_shortcut(text).unwrap();
        let profile = profile_for_shortcut(&config, &pressed("Shift+CommandOrControl+H")).unwrap();
        assert_eq!(profile.id.as_deref(), Some("zh-de"));
        assert_eq!(profile.source_lang, "zh-CN");

        let default = profile_for_shortcut(&config, &pressed("CommandOrControl+Shift+C")).unwrap();
        assert_eq!(default.id, None);

        assert_eq!(profile_for_shortcut(&config, &pressed("Alt+Q")), None);
    }
}
