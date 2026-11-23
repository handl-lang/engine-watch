//! `hnl-engine-watch` — a cross-platform filesystem watcher for the handl
//! toolchain.
//!
//! Phase 1 of the watch story (T-128): the Rust engine only. The `.hnl`
//! `lib-watch` binding that exposes this to Handl programs lands later.
//!
//! ```no_run
//! use hnl_engine_watch::{Watcher, WatchOptions, Filter};
//! use std::time::Duration;
//!
//! let watcher = Watcher::watch_with(
//!     "src",
//!     WatchOptions::new()
//!         .recursive(true)
//!         .with_filter(Filter::extension("rs")),
//! ).expect("watch src");
//!
//! // Debounced: one batch per burst of saves.
//! let debouncer = watcher.debounced(Duration::from_millis(100));
//! loop {
//!     let batch = debouncer.recv_batch().expect("watch src");
//!     println!("{} events", batch.len());
//! }
//! ```
//!
//! # Layout
//!
//! - [`Watcher`] — owns the OS backend (`notify`) and a channel of typed
//!   [`FsEvent`]s; `recv`/`recv_timeout`/`try_recv` consume it.
//! - [`WatchOptions`] / [`Filter`] — recursion flag plus an event predicate
//!   applied inside the backend handler (rejected events never queue).
//! - [`Debouncer`] — quiescence-window batching for rerun-style consumers.

mod debounce;
mod event;
mod watcher;

pub use debounce::Debouncer;
pub use event::{EventKind, FsEvent};
pub use watcher::{Filter, WatchOptions, Watcher};

use std::fmt;

/// Errors surfaced by [`Watcher`] and [`Debouncer`].
#[derive(Debug)]
pub enum Error {
    /// The OS backend reported an error (queue overflow, path vanished, …).
    Notify(notify::Error),
    /// `recv_timeout` elapsed with no event.
    Timeout,
    /// The watcher was dropped and the channel closed.
    Disconnected,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Notify(e) => write!(f, "watch backend error: {e}"),
            Error::Timeout => f.write_str("watch receive timed out"),
            Error::Disconnected => f.write_str("watch channel disconnected"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Notify(e) => Some(e),
            _ => None,
        }
    }
}

impl From<notify::Error> for Error {
    fn from(e: notify::Error) -> Self {
        Error::Notify(e)
    }
}
