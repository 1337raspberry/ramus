//! Home-screen widget: the snapshot the app publishes for it and the
//! commands its buttons send back.

pub mod actions;
pub mod ffi;
pub mod files;
pub mod publisher;

pub use publisher::WidgetPublisher;
