//! Casting: ramus as a Plex Companion controller. A cast plays ramus's queue
//! on another Plex player. The queue lives on the server as a play queue;
//! the player is commanded over HTTP and polled for its timeline.

pub mod companion;
pub mod play_queue;
pub mod players;
pub mod record;
pub mod session;
pub mod timeline;
