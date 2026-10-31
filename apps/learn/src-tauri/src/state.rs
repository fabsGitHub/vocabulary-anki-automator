//! Learn's runtime state, managed by Tauri and shared by every command.
//!
//! Everything that touches the outside world is injected through
//! [`AppServices`]: the database, the clock, the text-to-speech provider and
//! the curriculum to seed. Production wiring is [`AppServices::production`];
//! the `tauri::test` suites pass a temp-file database, a `FixedClock` and a
//! fake TTS, so no test reaches the network or the learner's real database.
//!
//! **Startup never fails.** [`AppState::new`] seeds `hsk_curriculum` with the
//! bundled curriculum, spends the streak freezes that missed days call for
//! and loads the settings, logging whatever goes wrong. A database that could
//! not be opened is then reported by every command that needs it, and a
//! curriculum that could not be seeded is retried, and reported, by the two
//! commands that read it (`get_daily_queue`, `start_new_item`); everything
//! else keeps working.
//!
//! **Locking** (docs/architecture.md § 6.3): the store sits behind a
//! `std::sync::Mutex` that is only held inside [`AppState::with_store`]'s
//! synchronous closure. Its guard is not `Send`, so holding it across an
//! `.await` (a TTS or AnkiConnect request) does not even compile into the
//! `Send` futures Tauri commands need, and no transaction opened inside the
//! closure can outlive it. The curriculum seed state and the settings are
//! only ever locked on their own or while the store already is, so the locks
//! are always taken in the same order.
//!
//! **One clock reading per command.** [`AppState::clock`] reads the injected
//! clock once, and a command hands that reading to every core call it makes:
//! a review, its XP and the session progress it counts towards then always
//! fall on the same local date, even when the command runs across midnight.

use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, PoisonError,
    },
};

use chrono::{DateTime, FixedOffset, Utc};
use vocab_core::{
    curriculum::{bundled_curriculum, CurriculumIndex},
    gamification::apply_streak_freezes,
    providers::{GoogleTtsProvider, TtsProvider},
    srs::SrsScheduler,
    store::Store,
    time::{Clock, SystemClock},
    CurriculumItem,
};

use crate::{
    error::Error,
    settings::{self, LearnSettings},
};

/// What [`AppState::new`] is built from.
pub struct AppServices {
    /// The shared database, or why it could not be opened.
    pub store: Result<Store, String>,
    pub clock: Arc<dyn Clock>,
    /// Speaks cards that have no stored recording, and example sentences.
    pub tts: Arc<dyn TtsProvider>,
    /// What `hsk_curriculum` must hold, or why it could not be loaded.
    pub curriculum: Result<Vec<CurriculumItem>, String>,
}

impl AppServices {
    /// The real thing: the database at `db_path`, the system clock, Google
    /// TTS and the curriculum compiled into vocab-core.
    pub fn production(db_path: &Path) -> Self {
        let store = Store::open(db_path).map_err(|err| {
            log::error!(
                "Could not open the vocabulary database at {}: {err}",
                db_path.display()
            );
            format!("could not open {}: {err}", db_path.display())
        });
        if store.is_ok() {
            log::info!("Vocabulary database: {}", db_path.display());
        }
        Self {
            store,
            clock: Arc::new(SystemClock),
            tts: Arc::new(GoogleTtsProvider::new()),
            curriculum: bundled_curriculum()
                .map_err(|err| format!("the bundled curriculum does not parse: {err}")),
        }
    }
}

/// Whether `hsk_curriculum` holds the curriculum yet.
enum Seed {
    Done,
    /// Seeding failed (why); the items are kept for the next attempt.
    Pending(Vec<CurriculumItem>, String),
    /// There is nothing to seed with.
    Unavailable(String),
}

/// Managed by Tauri; see the module docs.
pub struct AppState {
    store: Result<Mutex<Store>, String>,
    scheduler: SrsScheduler,
    clock: Arc<dyn Clock>,
    tts: Arc<dyn TtsProvider>,
    curriculum: Arc<CurriculumIndex>,
    seed: Mutex<Seed>,
    settings: Mutex<LearnSettings>,
    syncing_anki: AtomicBool,
}

impl AppState {
    /// Runs the startup steps of the module docs on `services`.
    pub fn new(services: AppServices) -> Self {
        let clock = services.clock;
        let mut store = services.store;
        let curriculum = match &services.curriculum {
            Ok(items) => CurriculumIndex::new(items.iter().cloned()),
            Err(_) => CurriculumIndex::default(),
        };
        let seed = match (services.curriculum, store.as_mut()) {
            (Err(why), _) => {
                log::error!("No HSK curriculum: {why}");
                Seed::Unavailable(why)
            }
            (Ok(items), Ok(store)) => seed_curriculum(store, items),
            (Ok(items), Err(why)) => Seed::Pending(items, why.clone()),
        };
        let mut settings = LearnSettings::default();
        if let Ok(store) = &store {
            let now = ClockSnapshot::read(clock.as_ref());
            match apply_streak_freezes(store, &now) {
                Ok(days) if !days.is_empty() => {
                    log::info!("Streak freezes covered {} missed day(s).", days.len());
                }
                Ok(_) => {}
                Err(err) => log::warn!("Could not apply streak freezes: {err}"),
            }
            match settings::load(store) {
                Ok(loaded) => settings = loaded,
                Err(err) => log::error!("Using the default settings: {err}"),
            }
        }
        Self {
            store: store.map(Mutex::new),
            scheduler: SrsScheduler::default(),
            clock,
            tts: services.tts,
            curriculum: Arc::new(curriculum),
            seed: Mutex::new(seed),
            settings: Mutex::new(settings),
            syncing_anki: AtomicBool::new(false),
        }
    }

    /// Runs `f` against the store, synchronously; see the module docs.
    pub fn with_store<T, E: Into<Error>>(
        &self,
        f: impl FnOnce(&Store) -> Result<T, E>,
    ) -> Result<T, Error> {
        self.with_store_mut(|store| f(store))
    }

    /// The error [`with_store`](Self::with_store) would give when there is no
    /// database, for commands that would otherwise ask the network first.
    pub fn require_store(&self) -> Result<(), Error> {
        self.with_store(|_| Ok::<_, Error>(()))
    }

    /// [`with_store`](Self::with_store) for the few writes that need the
    /// store exclusively (seeding the curriculum).
    pub fn with_store_mut<T, E: Into<Error>>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T, E>,
    ) -> Result<T, Error> {
        let store = self
            .store
            .as_ref()
            .map_err(|why| Error::StoreUnavailable(why.clone()))?;
        f(&mut lock(store)).map_err(Into::into)
    }

    /// Makes sure `hsk_curriculum` was seeded, trying again if startup could
    /// not do it. Call it with the store from [`with_store_mut`](Self::with_store_mut).
    pub fn ensure_curriculum(&self, store: &mut Store) -> Result<(), Error> {
        let mut seed = lock(&self.seed);
        if let Seed::Pending(items, _) = &mut *seed {
            *seed = seed_curriculum(store, std::mem::take(items));
        }
        match &*seed {
            Seed::Done => Ok(()),
            Seed::Pending(_, why) | Seed::Unavailable(why) => {
                Err(Error::CurriculumUnavailable(why.clone()))
            }
        }
    }

    /// The clock, read once: see the module docs.
    pub fn clock(&self) -> ClockSnapshot {
        ClockSnapshot::read(self.clock.as_ref())
    }

    pub fn scheduler(&self) -> &SrsScheduler {
        &self.scheduler
    }

    pub fn tts(&self) -> &dyn TtsProvider {
        self.tts.as_ref()
    }

    /// The bundled curriculum in memory, for annotating imported hanzi.
    pub fn curriculum(&self) -> Arc<CurriculumIndex> {
        self.curriculum.clone()
    }

    /// A snapshot of the current settings.
    pub fn settings(&self) -> LearnSettings {
        lock(&self.settings).clone()
    }

    /// Normalizes, validates and stores `settings`; memory is updated only
    /// once the database has them, under the store's lock, so two saves
    /// cannot leave memory and database disagreeing.
    pub fn save_settings(&self, mut settings: LearnSettings) -> Result<(), Error> {
        settings.normalize();
        settings.validate()?;
        self.with_store(|store| {
            settings::save(store, &settings)?;
            *lock(&self.settings) = settings;
            Ok::<_, Error>(())
        })
    }

    /// Marks an Anki export or import as running until the guard drops;
    /// `None` when one already is. Two of them side by side would push or
    /// insert the same cards twice.
    pub fn begin_anki_sync(&self) -> Option<FlagGuard<'_>> {
        FlagGuard::acquire(&self.syncing_anki)
    }
}

/// Seeds `hsk_curriculum` with `items` (a no-op when it already holds them).
fn seed_curriculum(store: &mut Store, items: Vec<CurriculumItem>) -> Seed {
    match store.ensure_curriculum_seeded(&items) {
        Ok(true) => {
            log::info!("Seeded the HSK curriculum ({} words).", items.len());
            Seed::Done
        }
        Ok(false) => Seed::Done,
        Err(err) => {
            log::error!("Could not seed the HSK curriculum: {err}");
            Seed::Pending(items, err.to_string())
        }
    }
}

/// One reading of a [`Clock`], which is itself a clock that stays at that
/// reading: see the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSnapshot {
    now: DateTime<Utc>,
    offset: FixedOffset,
}

impl ClockSnapshot {
    pub fn read(clock: &dyn Clock) -> Self {
        Self {
            now: clock.now(),
            offset: clock.local_offset(),
        }
    }
}

impl Clock for ClockSnapshot {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn local_offset(&self) -> FixedOffset {
        self.offset
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
/// data (a SQLite connection, a settings snapshot) is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::{Duration, TimeZone};
    use vocab_core::{time::FixedClock, ProviderError};

    use super::*;

    struct NoTts;

    #[async_trait]
    impl TtsProvider for NoTts {
        fn name(&self) -> &'static str {
            "no_tts"
        }

        async fn synthesize(&self, _: &str, _: &str) -> Result<Vec<u8>, ProviderError> {
            Err(ProviderError::Unsupported {
                provider: "no_tts".into(),
                what: "anything".into(),
            })
        }
    }

    fn word(hanzi: &str, order: i64) -> CurriculumItem {
        CurriculumItem {
            hanzi: hanzi.into(),
            traditional: None,
            hsk_level: 1,
            frequency: None,
            curriculum_order: order,
            pinyin: "x".into(),
            pos: None,
            meaning_en: Some("x".into()),
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

    fn state(store: Result<Store, String>, curriculum: Vec<CurriculumItem>) -> AppState {
        AppState::new(AppServices {
            store,
            clock: Arc::new(FixedClock::utc(
                Utc.with_ymd_and_hms(2026, 10, 31, 12, 0, 0).unwrap(),
            )),
            tts: Arc::new(NoTts),
            curriculum: Ok(curriculum),
        })
    }

    #[test]
    fn a_snapshot_keeps_the_reading_while_the_clock_moves_on() {
        let clock = FixedClock::new(
            Utc.with_ymd_and_hms(2026, 10, 31, 22, 59, 59).unwrap(),
            FixedOffset::east_opt(3600).unwrap(),
        );
        let snapshot = ClockSnapshot::read(&clock);
        clock.advance(Duration::seconds(2));

        assert_eq!(snapshot.today().to_string(), "2026-10-31");
        assert_eq!(clock.today().to_string(), "2026-11-01");
        assert_eq!(snapshot.local_offset(), clock.local_offset());
    }

    #[test]
    fn a_curriculum_that_could_not_be_seeded_is_retried_later() {
        // Two items with the same order make seeding fail every time.
        let state = state(
            Store::open_in_memory().map_err(|err| err.to_string()),
            vec![word("学习", 1), word("中文", 1)],
        );
        let err = state
            .with_store_mut(|store| state.ensure_curriculum(store))
            .unwrap_err();
        assert!(
            err.to_string()
                .starts_with("The HSK curriculum is unavailable"),
            "{err}"
        );
        assert_eq!(
            state.with_store(|store| store.curriculum_count()).unwrap(),
            0
        );

        // Once the items can be written, the retry fills the table.
        *lock(&state.seed) = Seed::Pending(vec![word("学习", 1), word("中文", 2)], String::new());
        state
            .with_store_mut(|store| state.ensure_curriculum(store))
            .unwrap();
        assert_eq!(
            state.with_store(|store| store.curriculum_count()).unwrap(),
            2
        );
    }

    #[test]
    fn without_a_database_commands_say_why() {
        let state = state(Err("simulated".into()), vec![word("学习", 1)]);
        let err = state
            .with_store(|store| store.curriculum_count())
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "The vocabulary database is unavailable (simulated)."
        );
        assert_eq!(state.settings(), LearnSettings::default());
        assert!(state.save_settings(LearnSettings::default()).is_err());
    }

    #[test]
    fn saving_invalid_settings_changes_nothing() {
        let state = state(
            Store::open_in_memory().map_err(|err| err.to_string()),
            vec![],
        );
        let mut settings = LearnSettings::default();
        settings.anki.deck_name = "   ".into();

        assert!(matches!(
            state.save_settings(settings),
            Err(Error::InvalidSettings(_))
        ));
        assert_eq!(state.settings(), LearnSettings::default());
        assert_eq!(
            state
                .with_store(|store| store.get_state(settings::SETTINGS_KEY))
                .unwrap(),
            None
        );
    }

    #[test]
    fn only_one_anki_sync_runs_at_a_time() {
        let state = state(Err("not needed".into()), vec![]);
        let guard = state.begin_anki_sync().expect("first sync");
        assert!(state.begin_anki_sync().is_none());
        drop(guard);
        assert!(state.begin_anki_sync().is_some());
    }
}
