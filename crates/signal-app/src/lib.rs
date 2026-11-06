//! Signal desktop client UI built with GPUI Kit.

pub mod actions;
pub mod app;
pub mod assets;
pub mod attachments;
mod chat_actions;
pub mod theme;
pub mod views;

pub use app::{CommandSink, DesktopNotifier, Notifier, RecordingSink, SignalApp};
