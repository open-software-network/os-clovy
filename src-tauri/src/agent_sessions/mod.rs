//! Clovy-owned agent persistence and legacy import foundations.

pub mod api;
pub mod domain;
pub mod events;
pub mod home_import;
pub mod migration;
pub mod persona;
pub mod protocol;
pub mod repository;
pub(crate) mod retirement;
pub(crate) mod run_partition;
pub(crate) mod skill_catalog;
pub(crate) mod skill_identity;
mod skill_writer;
pub mod skills;
pub(crate) mod terminal;

pub use domain::*;
pub use migration::{
    import_legacy_agent_state, legacy_import_completed, stop_legacy_hermes_runtime,
    LegacyImportError, LegacyImportOptions,
};
pub use repository::AgentRepository;
