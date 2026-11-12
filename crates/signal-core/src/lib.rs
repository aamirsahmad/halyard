//! Core of the Halyard client: domain model, encrypted local database,
//! UI state reducer and presentation helpers. This crate has no dependency on
//! the network layer nor on the UI toolkit, which keeps it fully unit-testable.

pub mod attachments;
pub mod avatar;
pub mod db;
pub mod format;
pub mod ids;
pub mod mentions;
pub mod model;
pub mod protocol;
pub mod richtext;
pub mod state;
pub mod timeline;

pub use db::Database;
pub use ids::{ConversationId, GroupKey};
pub use model::*;
pub use protocol::*;
pub use state::ChatState;
