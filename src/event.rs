//! Typed filesystem events produced by [`crate::Watcher`].

use std::path::PathBuf;

/// The coarse operation an [`FsEvent`] describes.
///
/// This is a normalized view over `notify::EventKind`: platform backends
/// spell the same logical change differently (inotify `MOVED_FROM`/`MOVED_TO`
/// pairs, FSEvents flags, ReadDirectoryChangesW action codes), and consumers
/// should match on this enum rather than the [`FsEvent::raw`] kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A path was created (file or directory).
    Create,
    /// A path's contents or metadata changed.
    Modify,
    /// A path was removed.
    Delete,
    /// A path was renamed or moved. Carries both endpoints — see
    /// [`FsEvent::path`] (source) and [`FsEvent::to`] (destination).
    Rename,
    /// Anything the backend reported that does not map cleanly onto the
    /// variants above (access events, unmounts, other/any-kind events).
    Other,
}

/// A single filesystem event.
#[derive(Debug, Clone)]
pub struct FsEvent {
    /// The normalized operation kind.
    pub kind: EventKind,
    /// The primary path. For [`EventKind::Rename`] this is the source path.
    pub path: PathBuf,
    /// The destination path for [`EventKind::Rename`] events where the
    /// backend reported both endpoints; `None` otherwise (and for renames
    /// the OS split into separate from/to notifications).
    pub to: Option<PathBuf>,
    /// The unmodified `notify` event kind, kept for consumers that need
    /// finer granularity (e.g. distinguishing data writes from metadata
    /// changes inside [`EventKind::Modify`]).
    pub raw: notify::EventKind,
}

impl FsEvent {
    /// Classify a `notify` event into an [`FsEvent`].
    ///
    /// Rename detection: `Modify(Name(RenameMode::Both))` carries both
    /// endpoints; `Modify(Name(_))` (From/To/Any/Other) is still reported as
    /// [`EventKind::Rename`] but without a paired `to`.
    pub(crate) fn from_notify(event: notify::Event) -> Self {
        use notify::event::{ModifyKind, RenameMode};

        let raw = event.kind;
        let kind = match &raw {
            notify::EventKind::Create(_) => EventKind::Create,
            notify::EventKind::Modify(ModifyKind::Name(mode)) => {
                // Every rename flavour maps to Rename; only `Both` gives us
                // the destination path.
                let _ = mode;
                EventKind::Rename
            }
            notify::EventKind::Modify(_) => EventKind::Modify,
            notify::EventKind::Remove(_) => EventKind::Delete,
            notify::EventKind::Access(_) | notify::EventKind::Other | notify::EventKind::Any => {
                EventKind::Other
            }
        };

        let mut paths = event.paths.into_iter();
        let path = paths.next().unwrap_or_default();
        let to = if matches!(
            raw,
            notify::EventKind::Modify(ModifyKind::Name(RenameMode::Both))
        ) {
            paths.next()
        } else {
            None
        };

        FsEvent {
            kind,
            path,
            to,
            raw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode};

    fn event(kind: notify::EventKind, paths: Vec<PathBuf>) -> FsEvent {
        let ev = notify::Event::new(kind);
        let ev = paths.into_iter().fold(ev, |ev, p| ev.add_path(p));
        FsEvent::from_notify(ev)
    }

    #[test]
    fn classifies_create_modify_delete() {
        let e = event(
            notify::EventKind::Create(CreateKind::File),
            vec!["/a/b.txt".into()],
        );
        assert_eq!(e.kind, EventKind::Create);
        assert_eq!(e.path, PathBuf::from("/a/b.txt"));
        assert_eq!(e.to, None);

        let e = event(
            notify::EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            vec!["/a/b.txt".into()],
        );
        assert_eq!(e.kind, EventKind::Modify);

        let e = event(
            notify::EventKind::Remove(RemoveKind::File),
            vec!["/a/b.txt".into()],
        );
        assert_eq!(e.kind, EventKind::Delete);
    }

    #[test]
    fn rename_both_carries_destination() {
        let e = event(
            notify::EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            vec!["/a/old.txt".into(), "/a/new.txt".into()],
        );
        assert_eq!(e.kind, EventKind::Rename);
        assert_eq!(e.path, PathBuf::from("/a/old.txt"));
        assert_eq!(e.to, Some(PathBuf::from("/a/new.txt")));
    }

    #[test]
    fn rename_from_without_pair_has_no_destination() {
        let e = event(
            notify::EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            vec!["/a/old.txt".into()],
        );
        assert_eq!(e.kind, EventKind::Rename);
        assert_eq!(e.to, None);
    }

    #[test]
    fn access_maps_to_other() {
        let e = event(
            notify::EventKind::Access(notify::event::AccessKind::Read),
            vec!["/a/b.txt".into()],
        );
        assert_eq!(e.kind, EventKind::Other);
    }
}
