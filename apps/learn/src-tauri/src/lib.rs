//! `Learn` — review sessions and quizzes over the shared vocabulary store
//! (Milestone I, decision 10). Commands are implemented in a later step; see
//! docs/architecture.md § 10.

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running the Learn application");
}
