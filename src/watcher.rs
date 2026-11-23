//! Filesystem watching: [`Watcher`] plus [`Filter`] and [`WatchOptions`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::event::FsEvent;
use crate::{Debouncer, Error};

/// Predicate applied to every event before it reaches the consumer.
///
/// Filters run inside the backend's event handler, so rejected events never
/// enter the channel — a noisy tree (e.g. `target/`, `.git/`) can be excluded
/// without the queue growing unboundedly.
pub struct Filter {
    pred: Arc<dyn Fn(&FsEvent) -> bool + Send + Sync>,
}

impl Filter {
    /// A filter from an arbitrary predicate.
    pub fn new<F>(pred: F) -> Self
    where
        F: Fn(&FsEvent) -> bool + Send + Sync + 'static,
    {
        Filter {
            pred: Arc::new(pred),
        }
    }

    /// Accept only events whose path has the given extension (case-sensitive,
    /// compared without the leading dot: `"rs"`, not `".rs"`).
    pub fn extension(ext: &str) -> Self {
        let ext = ext.trim_start_matches('.').to_owned();
        Filter::new(move |e| {
            let has_ext = |p: &Path| p.extension().is_some_and(|x| x == ext.as_str());
            has_ext(&e.path) || e.to.as_deref().is_some_and(has_ext)
        })
    }

    /// Accept only events under one of the given directory prefixes.
    pub fn under(roots: Vec<PathBuf>) -> Self {
        Filter::new(move |e| {
            let under_root = |p: &Path| roots.iter().any(|r| p.starts_with(r));
            under_root(&e.path) || e.to.as_deref().is_some_and(under_root)
        })
    }

    /// Accept every event.
    pub fn any() -> Self {
        Filter::new(|_| true)
    }

    pub(crate) fn accepts(&self, e: &FsEvent) -> bool {
        (self.pred)(e)
    }
}

impl std::fmt::Debug for Filter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Filter(..)")
    }
}

/// Options for [`Watcher::watch_with`] / [`Watcher::watch_many`].
pub struct WatchOptions {
    /// Watch directories recursively. `true` by default — the common case for
    /// dev servers and `--watch` modes.
    pub recursive: bool,
    /// Optional event predicate; `None` accepts everything.
    pub filter: Option<Filter>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        WatchOptions {
            recursive: true,
            filter: None,
        }
    }
}

impl WatchOptions {
    /// Recursive watching, no filter — same as [`Watcher::watch`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Non-recursive watching of a single directory.
    pub fn shallow() -> Self {
        WatchOptions {
            recursive: false,
            filter: None,
        }
    }

    /// Attach a filter.
    pub fn with_filter(mut self, filter: Filter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Set recursion explicitly.
    pub fn recursive(mut self, recursive: bool) -> Self {
        self.recursive = recursive;
        self
    }
}

/// Messages crossing the backend → consumer channel.
enum Msg {
    Event(FsEvent),
    Error(notify::Error),
}

/// A filesystem watcher over one or more root paths.
///
/// Events are consumed through a channel: [`Watcher::recv`] blocks for the
/// next [`FsEvent`], [`Watcher::recv_timeout`] bounds the wait, and
/// [`Watcher::try_recv`] polls. For burst-prone trees use
/// [`Debouncer`](crate::Debouncer) to collect quiescence-window batches.
///
/// Dropping the watcher stops the backend and disconnects the channel.
pub struct Watcher {
    inner: RecommendedWatcher,
    rx: Receiver<Msg>,
}

impl Watcher {
    /// Watch `path` recursively with no filter.
    pub fn watch<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        Self::watch_with(path, WatchOptions::new())
    }

    /// Watch `path` with the given [`WatchOptions`].
    pub fn watch_with<P: AsRef<Path>>(path: P, opts: WatchOptions) -> Result<Self, Error> {
        Self::watch_many([path.as_ref().to_path_buf()], opts)
    }

    /// Watch several roots with one backend watcher (one channel, one queue).
    /// `opts.recursive` applies to every root.
    pub fn watch_many<I, P>(paths: I, opts: WatchOptions) -> Result<Self, Error>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let (tx, rx) = channel::<Msg>();
        let filter = opts.filter.map(Arc::new);

        // The handler runs on the backend's thread: classify, filter, forward.
        // Notify errors are forwarded too so `recv` can surface them instead
        // of silently dropping the stream.
        let handler = move |res: notify::Result<notify::Event>| match res {
            Ok(ev) => {
                let fe = FsEvent::from_notify(ev);
                let accepted = filter.as_ref().is_none_or(|f| f.accepts(&fe));
                if accepted {
                    let _ = tx.send(Msg::Event(fe));
                }
            }
            Err(e) => {
                let _ = tx.send(Msg::Error(e));
            }
        };

        let mut inner = notify::recommended_watcher(handler)?;
        let mode = if opts.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        for path in paths {
            inner.watch(path.as_ref(), mode)?;
        }

        Ok(Watcher { inner, rx })
    }

    /// Add another root after construction.
    pub fn watch_path<P: AsRef<Path>>(&mut self, path: P, recursive: bool) -> Result<(), Error> {
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        Ok(self.inner.watch(path.as_ref(), mode)?)
    }

    /// Stop watching `path` (the watch is removed; the watcher keeps running).
    pub fn unwatch<P: AsRef<Path>>(&mut self, path: P) -> Result<(), Error> {
        Ok(self.inner.unwatch(path.as_ref())?)
    }

    /// Block until the next event (or a backend error).
    pub fn recv(&self) -> Result<FsEvent, Error> {
        match self.rx.recv() {
            Ok(Msg::Event(e)) => Ok(e),
            Ok(Msg::Error(e)) => Err(Error::Notify(e)),
            Err(_) => Err(Error::Disconnected),
        }
    }

    /// Block up to `timeout` for the next event.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<FsEvent, Error> {
        match self.rx.recv_timeout(timeout) {
            Ok(Msg::Event(e)) => Ok(e),
            Ok(Msg::Error(e)) => Err(Error::Notify(e)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(Error::Timeout),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(Error::Disconnected),
        }
    }

    /// Poll for an event without blocking.
    pub fn try_recv(&self) -> Result<FsEvent, Error> {
        self.recv_timeout(Duration::ZERO)
    }

    /// Consume this watcher into a [`Debouncer`] with the given quiescence
    /// window — the common "rerun the build when files settle" shape.
    pub fn debounced(self, window: Duration) -> Debouncer {
        Debouncer::new(self, window)
    }
}
