<!-- myco:agent-awareness -->

## Agent awareness

- Session start / any work discussion: run `myco status --prompt`
  before proposing next steps — know the board first.
- User describes new work → offer to capture it (`myco todo add`,
  `issue new`, `proposal new`). Don't wait to be asked.
- Picking up a task: `myco resume <ID>` is the briefing.
- Wrapping up: `myco handoff <ID>` anything resumable; keep statuses
  honest (claimed → in-progress → blocked/done).

<!-- /myco:agent-awareness -->

# Agent guide — tool-watch

`hnl-tool-watch` — the filesystem-watch engine: a `Watcher` that watches
paths (recursively by default) and yields typed `FsEvent`s
(`Create`/`Modify`/`Delete`/`Rename`/`Other`), with `WatchOptions`/`Filter`
predicates applied inside the backend handler and a `Debouncer` for
quiescence-window batching. Consumers: `tool-test --watch`, `ext-*`, and
future dev servers. Phase 1 of T-128 — the `.hnl` `lib-watch` binding is a
later phase and does not live here.

## Layout

- `src/event.rs` — `FsEvent`/`EventKind` classification over `notify` events.
- `src/watcher.rs` — `Watcher` (channel-fed `recv`/`recv_timeout`/`try_recv`),
  `WatchOptions`, `Filter`.
- `src/debounce.rs` — `Debouncer` (`recv_batch`, `recv_batch_timeout`).
- `tests/watch.rs` — tempdir watch cycles through the real OS backend.

## Contracts

- Backend is `notify` `8.2` — the only dependency. inotify on Linux,
  FSEvents/kqueue/ReadDirectoryChangesW elsewhere. Do not hand-roll
  platform backends; extend the typed layer (`event.rs`) instead.
- `publish = false` — git-install only, like every `tool-*`/`exec-*` crate.
- Crate name is `hnl-tool-watch`; repo is `handl-lang/tool-watch`.
- `WatchOptions::default()`/`new()` is **recursive** — `bool` fields need an
  explicit `Default` impl, `#[derive(Default)]` silently flips it to shallow.

## Verification

```sh
cargo xtask gate     # fmt --check → clippy -D warnings → test → doc
cargo test           # the suite
```

`ci.sh` is the bupha gate entry (`bash ci.sh` or `ci.sh fmt|clippy|test|doc`).
