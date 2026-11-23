//! Debounce/batch helper: collect a burst of events into one batch once the
//! stream goes quiet for a window.

use std::time::Duration;

use crate::event::FsEvent;
use crate::{Error, Watcher};

/// Batches [`FsEvent`]s by quiescence: after the first event arrives, keep
/// draining until no event has arrived for `window`, then yield the whole
/// burst as one `Vec<FsEvent>`.
///
/// This is the shape dev servers and `--watch` test runners want — a save
/// that touches five files produces one rerun, not five.
pub struct Debouncer {
    watcher: Watcher,
    window: Duration,
}

impl Debouncer {
    /// Wrap an existing [`Watcher`].
    pub fn new(watcher: Watcher, window: Duration) -> Self {
        Debouncer { watcher, window }
    }

    /// Unwrap back into the underlying [`Watcher`].
    pub fn into_watcher(self) -> Watcher {
        self.watcher
    }

    /// Block for the next batch. Returns `Err(Error::Timeout)` never — the
    /// first event is awaited indefinitely; the window only bounds the quiet
    /// period *after* activity starts. Returns `Err(Error::Disconnected)` when
    /// the watcher is dropped elsewhere.
    pub fn recv_batch(&self) -> Result<Vec<FsEvent>, Error> {
        let first = self.watcher.recv()?;
        let mut batch = vec![first];
        loop {
            match self.watcher.recv_timeout(self.window) {
                Ok(e) => batch.push(e),
                Err(Error::Timeout) => return Ok(batch),
                Err(e) => return Err(e),
            }
        }
    }

    /// Like [`Debouncer::recv_batch`], but the whole batch (including the
    /// first event) must complete within `max_wait`. Useful in tests and in
    /// shutdown paths where an unbounded block is unacceptable.
    pub fn recv_batch_timeout(&self, max_wait: Duration) -> Result<Vec<FsEvent>, Error> {
        let deadline = std::time::Instant::now() + max_wait;
        let first = self.watcher.recv_timeout(max_wait)?;
        let mut batch = vec![first];
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            match self.watcher.recv_timeout(remaining.min(self.window)) {
                Ok(e) => batch.push(e),
                Err(Error::Timeout) => {
                    // Distinguish "window elapsed" (done) from "max_wait hit"
                    // mid-burst: either way the batch is complete.
                    return Ok(batch);
                }
                Err(e) => return Err(e),
            }
        }
    }
}
