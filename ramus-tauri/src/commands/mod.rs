pub type CmdResult<T> = Result<T, String>;

pub mod acknowledgements;
pub mod auth;
pub mod library;
pub mod playback;
pub mod search;
pub mod settings;
pub mod spectrum;
pub mod sync;
