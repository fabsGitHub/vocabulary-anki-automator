//! `vocab-core` — the platform-agnostic core shared by `Automator`
//! (`src-tauri`) and `Learn` (`apps/learn`), per decisions 6 and 10.
//!
//! Module map (owners and contracts: docs/architecture.md):
//!
//! | module         | milestone | contents                                                     |
//! |----------------|-----------|--------------------------------------------------------------|
//! | `model`        | B, J1     | neutral `Card`, `ScriptMetadata`, `CurriculumItem`           |
//! | `error`        | C         | typed `ProviderError`                                        |
//! | `language`     | C, J4     | language-code normalization, CJK detection                   |
//! | `providers`    | B, C, J   | provider traits, MyMemory/Datamuse/dictionaryapi/TTS/Chinese |
//! | `pipeline`     | B, H, J4  | capture -> translate -> rarity -> example -> tts -> `Card`   |
//! | `curriculum`   | J2        | bundled HSK 3.0 data + in-memory lookup                      |
//! | `store`        | F, K      | SQLite schema, migrations, persistence                       |
//! | `srs`          | L         | FSRS scheduling, daily queue, daily onboarding prompt        |
//! | `quiz`         | M         | `LearnedVocabularyPool` and exercise generation              |
//! | `gamification` | I         | XP ledger, streaks and streak freezes                        |
//! | `lessons`      | I         | contextual lesson cards from captured example pairs          |
//! | `time`         | G+        | injectable `Clock`                                           |
//! | `rng`          | M         | deterministic Mulberry32 PRNG (Rust/TS parity)               |

pub mod curriculum;
pub mod error;
pub mod gamification;
pub mod language;
pub mod lessons;
pub mod model;
pub mod pipeline;
pub mod providers;
pub mod quiz;
pub mod rng;
pub mod srs;
pub mod store;
pub mod time;

pub use error::ProviderError;
pub use model::{
    CaptureOrigin, Card, CurriculumItem, ScriptMetadata, ScriptVariant, StoredCapture,
};
