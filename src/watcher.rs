//! Filesystem events for the workspace folders.
//!
//! The tree, the git marks and the open tabs are otherwise only refreshed on
//! explicit triggers — activation, save, a file operation, or Refresh — so a
//! `git checkout` or another editor's write is invisible until the user
//! refocuses the window. This module subscribes to the OS notifications
//! (FSEvents on macOS) and reports *which paths* changed; deciding what a
//! change means belongs to the app.
//!
//! Events are coarse and repeated, so the app debounces them. Watching is
//! recursive per folder, which is cheap on macOS and catches changes in
//! collapsed folders too.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use async_channel::Sender;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

/// What happened to a watched path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Change {
    /// The path was written, created, or renamed into place.
    Changed,
    /// The path was removed or renamed away.
    Removed,
}

impl Change {
    /// Maps a platform event kind. Only removal is called out: a reload of a
    /// path that is gone would fail, while every other kind is worth the same
    /// "read it again" treatment.
    fn of(kind: EventKind) -> Self {
        match kind {
            EventKind::Remove(_) => Self::Removed,
            _ => Self::Changed,
        }
    }
}

/// Watches a set of folders and forwards every changed path to a channel.
pub(crate) struct Watcher {
    inner: RecommendedWatcher,
    /// The folders currently watched, spelled the way the caller spelled them.
    watched: HashSet<PathBuf>,
    /// Canonical folder → caller's spelling. macOS reports events under
    /// canonical paths, while the tree's paths can go through a symlink.
    aliases: Arc<Mutex<Vec<(PathBuf, PathBuf)>>>,
}

impl Watcher {
    /// Builds a watcher that sends changed paths to `events`. `None` when the
    /// platform watcher cannot start; the app then stays pull-based, as before.
    pub(crate) fn new(events: Sender<(PathBuf, Change)>) -> Option<Self> {
        let aliases: Arc<Mutex<Vec<(PathBuf, PathBuf)>>> = Arc::default();
        let shared = aliases.clone();
        let inner = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let Ok(event) = result else {
                return;
            };
            let change = Change::of(event.kind);
            let aliases = shared.lock().unwrap_or_else(PoisonError::into_inner);
            for path in event.paths {
                let _ = events.try_send((rewrite(&path, &aliases), change));
            }
        })
        .ok()?;
        Some(Self {
            inner,
            watched: HashSet::new(),
            aliases,
        })
    }

    /// Follows exactly `paths`: new folders are added, the rest are dropped.
    pub(crate) fn sync(&mut self, paths: &[PathBuf]) {
        let wanted: HashSet<PathBuf> = paths.iter().cloned().collect();
        let stale: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|path| !wanted.contains(*path))
            .cloned()
            .collect();
        for path in stale {
            let _ = self.inner.unwatch(&path);
            self.watched.remove(&path);
        }
        for path in paths {
            if self.watched.contains(path) {
                continue;
            }
            // A folder can vanish between the caller deciding to watch it and
            // the watch being taken; that is not an error worth reporting.
            if self.inner.watch(path, RecursiveMode::Recursive).is_ok() {
                self.watched.insert(path.clone());
            }
        }

        let mut aliases = self.aliases.lock().unwrap_or_else(PoisonError::into_inner);
        aliases.clear();
        aliases.extend(self.watched.iter().map(|path| {
            (
                path.canonicalize().unwrap_or_else(|_| path.clone()),
                path.clone(),
            )
        }));
    }
}

/// `path` under the caller's spelling of the folder that contains it, so a
/// workspace folder reached through a symlink still matches its tree rows.
fn rewrite(path: &Path, aliases: &[(PathBuf, PathBuf)]) -> PathBuf {
    aliases
        .iter()
        .filter(|(canonical, _)| path.starts_with(canonical))
        .max_by_key(|(canonical, _)| canonical.components().count())
        .and_then(|(canonical, original)| {
            path.strip_prefix(canonical)
                .ok()
                .map(|rest| original.join(rest))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_canonical_event_path_is_rewritten_to_the_watched_spelling() {
        let aliases = vec![
            (
                PathBuf::from("/private/var/repo"),
                PathBuf::from("/var/repo"),
            ),
            (PathBuf::from("/private/var/other"), PathBuf::from("/other")),
        ];

        assert_eq!(
            rewrite(Path::new("/private/var/repo/src/main.rs"), &aliases),
            PathBuf::from("/var/repo/src/main.rs")
        );
        // The most specific alias wins, and unmatched paths pass through.
        assert_eq!(
            rewrite(Path::new("/private/var/repo"), &aliases),
            PathBuf::from("/var/repo")
        );
        assert_eq!(
            rewrite(Path::new("/elsewhere/file.rs"), &aliases),
            PathBuf::from("/elsewhere/file.rs")
        );
    }

    #[test]
    fn only_removals_are_called_out() {
        assert_eq!(
            Change::of(EventKind::Remove(notify::event::RemoveKind::File)),
            Change::Removed
        );
        for kind in [
            EventKind::Create(notify::event::CreateKind::File),
            EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Any,
            )),
            EventKind::Any,
        ] {
            assert_eq!(Change::of(kind), Change::Changed);
        }
    }

    #[test]
    fn a_file_created_in_a_watched_folder_is_reported() {
        let dir = std::env::temp_dir().join(format!("app-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let (sender, events) = async_channel::unbounded();
        let Some(mut watcher) = Watcher::new(sender) else {
            // No usable platform watcher here; nothing to verify.
            let _ = std::fs::remove_dir_all(&dir);
            return;
        };
        watcher.sync(std::slice::from_ref(&dir));
        std::fs::write(dir.join("new.txt"), "x").unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = false;
        while Instant::now() < deadline {
            if let Ok((path, _)) = events.try_recv() {
                // Coarse events may name the folder instead of the file, but
                // they must arrive in the caller's spelling of the folder.
                if path.starts_with(&dir) {
                    seen = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        let _ = std::fs::remove_dir_all(&dir);
        assert!(seen, "no event under {dir:?}");
    }
}
