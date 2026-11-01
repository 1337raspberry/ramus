//! Home-screen widget support: the state snapshot the widget renders from,
//! the colours it paints with, and the commands its buttons send back.

pub mod colors;
pub mod command;
pub mod snapshot;

pub use colors::*;
pub use command::*;
pub use snapshot::*;
