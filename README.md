# tool-watch

`hnl-tool-watch` — a cross-platform filesystem watcher: typed events,
filters, and debounce batching.

A `tool-*` crate: the watch operation implemented generically — consumers
(`tool-test --watch`, `ext-*` integrations, dev servers) get one typed
event stream regardless of whether the OS backend is inotify, FSEvents,
kqueue, or ReadDirectoryChangesW.

## API

- `Watcher::watch(path)` / `watch_with(path, WatchOptions)` /
  `watch_many(paths, opts)` — start watching; `recv()`, `recv_timeout(d)`,
  `try_recv()` consume typed `FsEvent`s.
- `FsEvent { kind, path, to, raw }` — `kind` is `Create` / `Modify` /
  `Delete` / `Rename` / `Other`; renames carry both endpoints (`path` =
  source, `to` = destination when the backend reports the pair).
- `WatchOptions { recursive, filter }` + `Filter` — recursion flag and a
  predicate applied inside the backend handler, so rejected events never
  enter the queue. Helpers: `Filter::extension`, `Filter::under`,
  `Filter::any`, `Filter::new`.
- `watcher.debounced(window)` → `Debouncer` — quiescence-window batching:
  `recv_batch()` returns one `Vec<FsEvent>` per burst of activity.

## Backend

Built on [`notify`](https://crates.io/crates/notify) `8.2` (inotify /
FSEvents / kqueue / ReadDirectoryChangesW; `8.2.2` is the newest 8.x stable —
`9.0.0` was <7 days old at pin time). `notify` is the only dependency; the
crate is `publish = false`, git-install only like the rest of the family.

The `.hnl` `lib-watch` binding is a later phase (T-128) and deliberately not
part of this crate.

## Layout

```text
src/event.rs      FsEvent + EventKind classification over notify events
src/watcher.rs    Watcher, WatchOptions, Filter
src/debounce.rs   Debouncer (quiescence-window batching)
tests/watch.rs    tempdir create/modify/delete/rename cycles through the OS backend
xtask/            cargo xtask gate (fmt → clippy → test → doc)
```

## Gate

```bash
cargo xtask gate        # fmt --check → clippy -D warnings → test → doc
```

`cargo xtask` wraps the repo's `xtask/` crate (`fmt`, `clippy`, `test`, `doc`,
`check`, `gate`); `ci.sh` is the same gate entry for the bupha shell runner.

## License

MIT.
