//! The Automator's runtime state, managed by Tauri and shared by every
//! command, the hotkey handler and the capture task.
//!
//! Everything that touches the outside world is injected through
//! [`AppServices`]: where the config lives, the database, the enrichment
//! providers, the clock, and Anki's folders plus the downloader and process
//! runner used by setup. Production wiring is [`AppServices::production`];
//! the `tauri::test` suites pass temp dirs and fakes, so no test can reach the
//! network, a real Anki folder or the real config.
//!
//! Locking rules (docs/architecture.md § 6.3): the store is behind a
//! `std::sync::Mutex` that is only ever held inside [`AppState::with_store`]'s
//! synchronous closure. Its guard is not `Send`, so holding it across an
//! `.await` (an AnkiConnect or TTS request) does not even compile into the
//! `Send` futures Tauri commands need, and neither can a transaction opened in
//! that closure outlive it.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
};

use chrono::{DateTime, SubsecRound, Utc};
use vocab_core::{
    store::{default_db_path, Store, StoreResult},
    time::{Clock, SystemClock},
};

use crate::{
    capture::{PendingCapture, ProviderFactory, StandardProviderFactory},
    config::{load_config_from, save_config_to, AppConfig, ConfigError},
    setup::SetupEnv,
};

/// What [`AppState::new`] is built from.
pub struct AppServices {
    pub config_path: PathBuf,
    /// The shared database, or why it could not be opened (the app then runs
    /// in Anki-only mode).
    pub store: Result<Store, String>,
    pub providers: Arc<dyn ProviderFactory>,
    pub clock: Arc<dyn Clock>,
    pub setup: SetupEnv,
}

impl AppServices {
    /// The real thing: `config_path`, the shared database at
    /// [`default_db_path`], the standard providers, the system clock and the
    /// platform's Anki folders.
    pub fn production(config_path: PathBuf) -> Self {
        let db_path = default_db_path();
        let store = Store::open(&db_path).map_err(|err| {
            log::error!(
                "Could not open the capture history at {}: {err}. Cards will only go to Anki.",
                db_path.display()
            );
            format!("could not open {}: {err}", db_path.display())
        });
        if store.is_ok() {
            log::info!("Capture history: {}", db_path.display());
        }
        Self {
            config_path,
            store,
            providers: Arc::new(StandardProviderFactory::default()),
            clock: Arc::new(SystemClock),
            setup: SetupEnv::production(),
        }
    }
}

/// Managed by Tauri; see the module docs.
pub struct AppState {
    config_path: PathBuf,
    config: Mutex<AppConfig>,
    store: Result<Mutex<Store>, String>,
    pending: PendingPayload,
    capturing: AtomicBool,
    submitting: AtomicBool,
    sync_lock: tokio::sync::Mutex<()>,
    providers: Arc<dyn ProviderFactory>,
    clock: Arc<dyn Clock>,
    setup: SetupEnv,
}

impl AppState {
    /// Loads the config from `services.config_path`. A file that cannot be
    /// read or parsed is copied to `config.json.bak` and replaced by the
    /// defaults in memory, so the app still starts and the user's file is
    /// still there to recover.
    pub fn new(services: AppServices) -> Self {
        let config = match load_config_from(&services.config_path) {
            Ok(config) => config,
            Err(err) => {
                log::error!("Using the default settings: {err}");
                if matches!(err, ConfigError::Parse { .. }) {
                    back_up_unreadable(&services.config_path);
                }
                AppConfig::default()
            }
        };
        Self {
            config_path: services.config_path,
            config: Mutex::new(config),
            store: services.store.map(Mutex::new),
            pending: PendingPayload::default(),
            capturing: AtomicBool::new(false),
            submitting: AtomicBool::new(false),
            sync_lock: tokio::sync::Mutex::new(()),
            providers: services.providers,
            clock: services.clock,
            setup: services.setup,
        }
    }

    /// A snapshot of the current settings.
    pub fn config(&self) -> AppConfig {
        lock(&self.config).clone()
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Applies `change` to the settings and saves them; memory is updated
    /// only once the file is written. Updates are serialized, so two of them
    /// cannot lose each other's changes.
    pub fn update_config<T>(
        &self,
        change: impl FnOnce(&mut AppConfig) -> T,
    ) -> Result<T, ConfigError> {
        let mut current = lock(&self.config);
        let mut next = current.clone();
        let value = change(&mut next);
        next.normalize();
        save_config_to(&self.config_path, &next)?;
        *current = next;
        Ok(value)
    }

    /// Runs `f` against the store, synchronously; see the module docs. Fails
    /// with a readable message in Anki-only mode or when `f` fails.
    pub fn with_store<T>(&self, f: impl FnOnce(&Store) -> StoreResult<T>) -> Result<T, String> {
        let store = self
            .store
            .as_ref()
            .map_err(|why| format!("The capture history is unavailable ({why})."))?;
        f(&lock(store)).map_err(|err| format!("Capture history error: {err}"))
    }

    /// Whether the shared database is open (false = Anki-only mode).
    pub fn has_store(&self) -> bool {
        self.store.is_ok()
    }

    pub fn pending(&self) -> &PendingPayload {
        &self.pending
    }

    /// Marks a capture as running until the guard drops; `None` when one
    /// already is.
    pub fn begin_capture(&self) -> Option<FlagGuard<'_>> {
        FlagGuard::acquire(&self.capturing)
    }

    /// True from the hotkey until the card is approved or discarded: while
    /// the pipeline runs and while a capture waits in the approval window.
    /// The updater checks this, so a restart never loses a card (N4).
    pub fn capture_in_progress(&self) -> bool {
        self.capturing.load(Ordering::SeqCst) || self.pending.is_set()
    }

    /// Marks a card submission as running until the guard drops; `None` when
    /// one already is (a double-clicked Approve must not save the card twice).
    pub fn begin_submit(&self) -> Option<FlagGuard<'_>> {
        FlagGuard::acquire(&self.submitting)
    }

    /// Held for the whole of `sync_pending_captures`, so two retries never
    /// push the same captures side by side.
    pub fn sync_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.sync_lock
    }

    pub fn providers(&self) -> &dyn ProviderFactory {
        self.providers.as_ref()
    }

    /// Now, to the millisecond: the precision the store keeps, so a pending
    /// card and its stored row carry the same `captured_at`.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now().trunc_subsecs(3)
    }

    pub fn setup(&self) -> &SetupEnv {
        &self.setup
    }
}

/// The capture waiting in the approval window (B3), with a generation number
/// so a submission only clears the capture it was started for, not a newer
/// one that arrived while it talked to Anki.
#[derive(Default)]
pub struct PendingPayload {
    current: Mutex<Option<(u64, PendingCapture)>>,
    generations: AtomicU64,
}

impl PendingPayload {
    /// Replaces the pending capture; returns its generation.
    pub fn set(&self, capture: PendingCapture) -> u64 {
        let generation = self.generations.fetch_add(1, Ordering::SeqCst) + 1;
        *lock(&self.current) = Some((generation, capture));
        generation
    }

    pub fn get(&self) -> Option<PendingCapture> {
        lock(&self.current)
            .as_ref()
            .map(|(_, capture)| capture.clone())
    }

    /// Whether a capture is pending, without copying it (and its audio).
    pub fn is_set(&self) -> bool {
        lock(&self.current).is_some()
    }

    /// The pending capture with its generation.
    pub fn snapshot(&self) -> Option<(u64, PendingCapture)> {
        lock(&self.current).clone()
    }

    pub fn clear(&self) {
        *lock(&self.current) = None;
    }

    /// Clears the pending capture if it is still `generation`.
    pub fn clear_if(&self, generation: u64) -> bool {
        let mut current = lock(&self.current);
        if current.as_ref().is_some_and(|(g, _)| *g == generation) {
            *current = None;
            true
        } else {
            false
        }
    }
}

/// Resets its flag when dropped, also when the task holding it panics.
pub struct FlagGuard<'a>(&'a AtomicBool);

impl<'a> FlagGuard<'a> {
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self(flag))
    }
}

impl Drop for FlagGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A poisoned lock only means another thread panicked while holding it; the
/// data (a config snapshot, a SQLite connection) is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn back_up_unreadable(path: &Path) {
    let backup = path.with_extension("json.bak");
    match std::fs::copy(path, &backup) {
        Ok(_) => log::warn!("Kept the unreadable settings file as {}.", backup.display()),
        Err(err) => log::error!("Could not back up {}: {err}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use vocab_core::{time::FixedClock, Card};

    use super::*;
    use crate::setup::SetupEnv;

    fn pending(front: &str) -> PendingCapture {
        PendingCapture {
            card: Card::new(front, "", "en", "de", Utc.timestamp_millis_opt(0).unwrap()),
            warnings: Vec::new(),
            profile_id: None,
        }
    }

    fn state_in(dir: &Path, clock: FixedClock) -> AppState {
        AppState::new(AppServices {
            config_path: dir.join("config.json"),
            store: Err("not needed".into()),
            providers: Arc::new(StandardProviderFactory::default()),
            clock: Arc::new(clock),
            setup: SetupEnv::unavailable("not needed"),
        })
    }

    #[test]
    fn clear_if_only_clears_the_capture_it_was_given() {
        let payload = PendingPayload::default();
        let first = payload.set(pending("first"));
        let second = payload.set(pending("second"));
        assert_ne!(first, second);

        assert!(!payload.clear_if(first), "a newer capture must survive");
        assert_eq!(payload.get().unwrap().card.front, "second");
        assert!(payload.clear_if(second));
        assert_eq!(payload.get(), None);
    }

    #[test]
    fn flag_guards_exclude_each_other_and_reset_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path(), FixedClock::utc(Utc::now()));
        let guard = state.begin_capture().expect("first capture");
        assert!(state.begin_capture().is_none());
        assert!(state.capture_in_progress());
        drop(guard);
        assert!(!state.capture_in_progress());
        assert!(state.begin_capture().is_some());
    }

    #[test]
    fn a_waiting_capture_counts_as_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path(), FixedClock::utc(Utc::now()));
        state.pending().set(pending("x"));
        assert!(state.capture_in_progress());
        state.pending().clear();
        assert!(!state.capture_in_progress());
    }

    #[test]
    fn now_is_truncated_to_milliseconds() {
        let dir = tempfile::tempdir().unwrap();
        let instant = Utc.timestamp_opt(1_793_448_000, 123_456_789).unwrap();
        let state = state_in(dir.path(), FixedClock::utc(instant));
        assert_eq!(state.now().timestamp_subsec_nanos(), 123_000_000);
    }

    #[test]
    fn an_unreadable_config_falls_back_to_defaults_and_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), "{ broken").unwrap();

        let state = state_in(dir.path(), FixedClock::utc(Utc::now()));

        assert_eq!(state.config(), AppConfig::default());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("config.json.bak")).unwrap(),
            "{ broken"
        );
    }

    #[test]
    fn update_config_saves_before_it_changes_memory() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path(), FixedClock::utc(Utc::now()));

        state
            .update_config(|config| config.ui_lang = "de".into())
            .unwrap();
        assert_eq!(state.config().ui_lang, "de");
        assert_eq!(load_config_from(state.config_path()).unwrap().ui_lang, "de");

        // Make the next write fail: the config path becomes a directory.
        std::fs::remove_file(state.config_path()).unwrap();
        std::fs::create_dir(state.config_path()).unwrap();
        std::fs::write(state.config_path().join("x"), "").unwrap();
        assert!(state
            .update_config(|config| config.ui_lang = "fr".into())
            .is_err());
        assert_eq!(state.config().ui_lang, "de", "memory keeps what is on disk");
    }

    #[test]
    fn anki_only_mode_explains_itself() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path(), FixedClock::utc(Utc::now()));
        assert!(!state.has_store());
        let err = state
            .with_store(|store| store.list_recent_captures(1))
            .unwrap_err();
        assert_eq!(err, "The capture history is unavailable (not needed).");
    }
}
