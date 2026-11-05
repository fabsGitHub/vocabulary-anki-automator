//! `hsk-import`: the one-time ETL behind
//! `crates/vocab-core/data/hsk_curriculum.json` (Milestones J2 and J2b).
//!
//! The apps never call any of this. Decision 9 ships the HSK 3.0 curriculum
//! as static data compiled into vocab-core, so the data is produced here, by
//! hand, from pinned upstream commits, and reviewed as a diff before it is
//! committed. The library/binary split exists only so the steps can be tested
//! without a network.
//!
//! | step          | module                 | does                                                  |
//! |---------------|------------------------|-------------------------------------------------------|
//! | `fetch`       | `sources`              | downloads the pinned files, verifies them by SHA-256  |
//! | `build`       | `curriculum`, `select` | flattens them into `CurriculumItem`s and reports why |
//! | `backfill-de` | `backfill`             | machine-translated German meanings, resumable         |
//! | `apkg-enrich` | `apkg`                 | sentences, audio and images from the user's Anki deck |
//!
//! Two upstreams are needed because neither is enough alone:
//! drkameleon/complete-hsk-vocabulary (`vocabulary`) has the meanings, but
//! lists every reading of a word in dictionary order; ivankra/hsk30
//! (`official`) is the official list, one row per reading, without meanings.
//! The official list decides which reading is the HSK word (`select`).
//! Both are MIT-licensed; data/README.md next to the bundled file records
//! provenance, licenses and the known gaps.

pub mod apkg;
pub mod backfill;
pub mod cli;
pub mod curriculum;
pub mod error;
pub mod official;
pub mod output;
pub mod reading;
pub mod select;
pub mod sources;
pub mod vocabulary;

pub use error::{Error, Result};
