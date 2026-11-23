//! Integration tests: real tempdir watch cycles through the OS backend.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hnl_tool_watch::{Error, EventKind, Filter, FsEvent, WatchOptions, Watcher};

/// Minimal tempdir (no dev-deps): unique dir under `std::env::temp_dir`,
/// removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("hnl-tool-watch-test-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Collect events for `window`, returning everything that arrived.
fn drain(w: &Watcher, window: Duration) -> Vec<FsEvent> {
    let mut out = Vec::new();
    let deadline = Instant::now() + window;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return out;
        }
        match w.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(e) => out.push(e),
            Err(Error::Timeout) => continue,
            Err(e) => panic!("watch error: {e}"),
        }
    }
}

/// Wait until an event matching `pred` arrives (or fail after `timeout`).
fn wait_for<F>(w: &Watcher, timeout: Duration, pred: F) -> FsEvent
where
    F: Fn(&FsEvent) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for event");
        match w.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(e) if pred(&e) => return e,
            Ok(_) => continue,
            Err(Error::Timeout) => continue,
            Err(e) => panic!("watch error: {e}"),
        }
    }
}

/// Let the OS backend settle after `watch()` before mutating the tree —
/// inotify watch registration is async on some backends.
fn settle() {
    std::thread::sleep(Duration::from_millis(150));
}

#[test]
fn create_modify_delete_cycle() {
    let dir = TempDir::new();
    let w = Watcher::watch(dir.path()).expect("watch tempdir");
    settle();

    let file = dir.path().join("a.txt");
    fs::write(&file, b"one").unwrap();
    let e = wait_for(&w, Duration::from_secs(5), |e| {
        e.kind == EventKind::Create && e.path == file
    });
    assert_eq!(e.kind, EventKind::Create);

    fs::write(&file, b"two").unwrap();
    let e = wait_for(&w, Duration::from_secs(5), |e| {
        e.kind == EventKind::Modify && e.path == file
    });
    assert_eq!(e.kind, EventKind::Modify);

    fs::remove_file(&file).unwrap();
    wait_for(&w, Duration::from_secs(5), |e| {
        e.kind == EventKind::Delete && e.path == file
    });
}

#[test]
fn rename_reports_both_endpoints() {
    let dir = TempDir::new();
    let src = dir.path().join("old.txt");
    let dst = dir.path().join("new.txt");
    fs::write(&src, b"x").unwrap();

    let w = Watcher::watch(dir.path()).expect("watch tempdir");
    settle();

    fs::rename(&src, &dst).unwrap();

    // On inotify a same-dir rename arrives as one Rename(Both) event. Other
    // backends may split it into from/to halves — accept either as long as a
    // Rename touching `src` shows up.
    wait_for(&w, Duration::from_secs(5), |e| {
        e.kind == EventKind::Rename
            && (e.path == src || e.path == dst || e.to.as_deref() == Some(dst.as_path()))
    });
}

#[test]
fn recursive_watch_sees_nested_paths() {
    let dir = TempDir::new();
    let nested = dir.path().join("sub").join("deep");
    fs::create_dir_all(&nested).unwrap();

    let w = Watcher::watch(dir.path()).expect("watch tempdir");
    settle();

    let file = nested.join("leaf.txt");
    fs::write(&file, b"deep").unwrap();
    wait_for(&w, Duration::from_secs(5), |e| e.path == file);
}

#[test]
fn shallow_watch_ignores_nested() {
    let dir = TempDir::new();
    let nested = dir.path().join("sub");
    fs::create_dir_all(&nested).unwrap();

    let w =
        Watcher::watch_with(dir.path(), WatchOptions::shallow()).expect("watch tempdir shallow");
    settle();

    // Top-level create is seen…
    let top = dir.path().join("top.txt");
    fs::write(&top, b"t").unwrap();
    wait_for(&w, Duration::from_secs(5), |e| e.path == top);

    // …but a create inside `sub/` is not.
    let inner = nested.join("inner.txt");
    fs::write(&inner, b"i").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        drain(&w, Duration::from_millis(200))
            .iter()
            .all(|e| e.path != inner),
        "shallow watch leaked a nested event"
    );
}

#[test]
fn filter_drops_rejected_events_at_the_source() {
    let dir = TempDir::new();
    let w = Watcher::watch_with(
        dir.path(),
        WatchOptions::new().with_filter(Filter::extension("rs")),
    )
    .expect("watch tempdir");
    settle();

    let keep = dir.path().join("keep.rs");
    let drop = dir.path().join("drop.txt");
    fs::write(&keep, b"k").unwrap();
    fs::write(&drop, b"d").unwrap();

    wait_for(&w, Duration::from_secs(5), |e| e.path == keep);
    std::thread::sleep(Duration::from_millis(400));
    let rest = drain(&w, Duration::from_millis(300));
    assert!(
        rest.iter().all(|e| e.path != drop),
        "filter leaked a rejected event: {rest:?}"
    );
}

#[test]
fn debouncer_batches_a_burst() {
    let dir = TempDir::new();
    let debouncer = Watcher::watch(dir.path())
        .expect("watch tempdir")
        .debounced(Duration::from_millis(150));
    settle();

    // A burst of writes inside the quiescence window.
    for i in 0..3 {
        fs::write(dir.path().join(format!("f{i}.txt")), b"x").unwrap();
    }

    let batch = debouncer
        .recv_batch_timeout(Duration::from_secs(5))
        .expect("batch");
    assert!(
        batch.len() >= 3,
        "expected the burst in one batch, got {} events: {batch:?}",
        batch.len()
    );
}

#[test]
fn watch_many_covers_multiple_roots() {
    let dir_a = TempDir::new();
    let dir_b = TempDir::new();
    let w = Watcher::watch_many(
        [dir_a.path().to_path_buf(), dir_b.path().to_path_buf()],
        WatchOptions::new(),
    )
    .expect("watch two roots");
    settle();

    let fa = dir_a.path().join("a.txt");
    let fb = dir_b.path().join("b.txt");
    fs::write(&fa, b"a").unwrap();
    fs::write(&fb, b"b").unwrap();

    wait_for(&w, Duration::from_secs(5), |e| e.path == fa);
    wait_for(&w, Duration::from_secs(5), |e| e.path == fb);
}

#[test]
fn try_recv_is_nonblocking() {
    let dir = TempDir::new();
    let w = Watcher::watch(dir.path()).expect("watch tempdir");
    settle();

    // Nothing written yet: poll returns Timeout, not a block.
    assert!(matches!(w.try_recv(), Err(Error::Timeout)));
}
