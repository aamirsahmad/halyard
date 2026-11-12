//! Signal network backend built on presage / libsignal.
//!
//! The backend runs on a dedicated thread and talks to the UI exclusively
//! through [`signal_core::BackendCommand`]s and [`signal_core::BackendEvent`]s.

pub mod demo;
pub mod groups;
pub mod interpret;
pub mod paths;
pub mod storage;
pub mod worker;

/// User agent for requests this crate makes itself.
pub const USER_AGENT: &str = concat!("halyard/", env!("CARGO_PKG_VERSION"));

pub use paths::DataPaths;
pub use presage;
pub use presage::libsignal_service::configuration::SignalServers;
pub use worker::{BackendConfig, BackendHandle, EventSink, spawn};
