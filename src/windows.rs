//! App-level state shared by every window: which windows are open, the recent
//! list, and the one place that writes `session.json`.
//!
//! [`TeakApp`] is one entity per window, so none of this can live on it: with
//! two windows, a per-window recent list or a per-window session file would
//! disagree. This module owns the shared half and the window bootstrap, so
//! `main` and the app's New Window command open a window the same way.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::*;

use crate::app::{TeakApp, WINDOW_TITLE};
use crate::session::{
    SessionState, WindowFrame, WindowFrameState, WindowSession, WorkspaceSession, push_recent,
    session_path,
};
use crate::settings::{Settings, load_settings};
use crate::workspace_file;

/// One open window: the platform handle, its app entity, and the id used in the
/// session file.
struct WindowEntry {
    id: u64,
    handle: AnyWindowHandle,
    app: WeakEntity<TeakApp>,
}

/// The app's live windows, in creation order.
#[derive(Default)]
pub(crate) struct WindowRegistry {
    entries: Vec<WindowEntry>,
    next_id: u64,
    /// The window the user last had focused, so the session can list it last
    /// and a relaunch focuses it again.
    active: u64,
    /// Set once a quit has been confirmed, so closing windows during teardown
    /// does not prompt again or remove their saved session state.
    quitting: bool,
}

impl WindowRegistry {
    /// Ids start at 1 so a default 0 is never a real window.
    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        id
    }

    /// Every window that is still alive, oldest first. Stale weak handles are
    /// skipped rather than removed so this can take `&self`.
    fn apps(&self) -> Vec<(u64, AnyWindowHandle, Entity<TeakApp>)> {
        self.entries
            .iter()
            .filter_map(|entry| entry.app.upgrade().map(|app| (entry.id, entry.handle, app)))
            .collect()
    }

    fn remove(&mut self, id: u64) {
        self.entries.retain(|entry| entry.id != id);
    }

    fn is_empty(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| entry.app.upgrade().is_none())
    }
}

/// The list behind File > Open Recent. One copy for the whole app.
#[derive(Default)]
pub(crate) struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    fn new(mut paths: Vec<PathBuf>) -> Self {
        paths.retain(|path| path.exists());
        Self { paths }
    }

    fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    fn note(&mut self, path: PathBuf) {
        push_recent(&mut self.paths, path);
    }

    fn clear(&mut self) {
        self.paths.clear();
    }
}

/// The one writer of `session.json`. Every window publishes its snapshot here;
/// writing the whole file from each window would let the last one win.
#[derive(Default)]
pub(crate) struct SessionStore {
    windows: BTreeMap<u64, WindowSession>,
    /// Parked tab sets, keyed by workspace file. Written by whichever window is
    /// showing that workspace, so two windows on one workspace resolve the way
    /// settings and the recent list do.
    workspaces: BTreeMap<PathBuf, WorkspaceSession>,
    recent: Vec<PathBuf>,
    active: u64,
}

impl SessionStore {
    fn set_window(&mut self, id: u64, window: WindowSession) {
        self.windows.insert(id, window);
    }

    /// Parks `state` under `path`. A window that switched away from its
    /// workspace passes the set it was showing; the last writer wins.
    fn set_workspace(&mut self, path: PathBuf, state: WorkspaceSession) {
        self.workspaces.insert(path, state);
    }

    /// The tab set last parked under `path`, for a window switching back to it.
    fn parked(&self, path: &Path) -> Option<WorkspaceSession> {
        self.workspaces.get(path).cloned()
    }

    fn remove_window(&mut self, id: u64) {
        self.windows.remove(&id);
        if self.active == id {
            self.active = 0;
        }
    }

    fn set_recent(&mut self, recent: Vec<PathBuf>) {
        self.recent = recent;
    }

    fn set_active(&mut self, active: u64) {
        self.active = active;
    }

    /// The windows, with the last-focused one at the end so opening them in
    /// order leaves that window focused.
    fn state(&self) -> SessionState {
        let mut windows = Vec::with_capacity(self.windows.len());
        for (id, window) in &self.windows {
            if *id != self.active {
                windows.push(window.clone());
            }
        }
        if let Some(window) = self.windows.get(&self.active) {
            windows.push(window.clone());
        }
        SessionState {
            windows,
            workspaces: self.workspaces.clone(),
            recent: self.recent.clone(),
        }
    }

    /// Writes the current state, atomically replacing the previous file. A
    /// missing support directory or a serialization failure is ignored, as it
    /// was when each window wrote its own session.
    fn write(&self) {
        let Some(path) = session_path() else {
            return;
        };
        let Ok(json) = serde_json::to_string_pretty(&self.state()) else {
            return;
        };
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let temp = path.with_file_name("session.json.tmp");
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }
}

/// The handles every window carries so it can reach app-level state.
#[derive(Clone)]
pub(crate) struct AppServices {
    registry: Entity<WindowRegistry>,
    recent: Entity<RecentFiles>,
    session: Entity<SessionStore>,
}

impl AppServices {
    /// Creates the shared entities. `recent` seeds the Open Recent menu from
    /// the session that was just loaded.
    pub(crate) fn new(cx: &mut App, recent: Vec<PathBuf>) -> Self {
        Self {
            registry: cx.new(|_| WindowRegistry::default()),
            recent: cx.new(|_| RecentFiles::new(recent)),
            session: cx.new(|_| SessionStore::default()),
        }
    }

    pub(crate) fn recent_paths(&self, cx: &App) -> Vec<PathBuf> {
        self.recent.read(cx).paths().to_vec()
    }

    /// The tabs the last window to show workspace `path` parked there. `None`
    /// means the workspace has not been open since the session was written, so
    /// switching to it starts with no tabs.
    pub(crate) fn parked_workspace(&self, path: &Path, cx: &App) -> Option<WorkspaceSession> {
        self.session.read(cx).parked(path)
    }

    /// True while at least one window is still alive. Used by the macOS reopen
    /// callback to decide whether a dock click needs a fresh window.
    pub(crate) fn has_windows(&self, cx: &App) -> bool {
        !self.registry.read(cx).is_empty()
    }

    /// Records the window the user last focused. Its snapshot is written last,
    /// so a relaunch opens it last and it keeps focus.
    pub(crate) fn mark_active(&self, id: u64, cx: &mut App) {
        self.registry.update(cx, |registry, _| registry.active = id);
    }

    fn active_window_id(&self, cx: &App) -> u64 {
        self.registry.read(cx).active
    }

    pub(crate) fn note_recent(&self, path: PathBuf, cx: &mut App) {
        self.recent.update(cx, |recent, _| recent.note(path));
    }

    pub(crate) fn clear_recent(&self, cx: &mut App) {
        self.recent.update(cx, |recent, _| recent.clear());
    }

    /// Publishes one window's snapshot and the tabs it has parked under a
    /// workspace, plus the shared recent list, and writes the file.
    pub(crate) fn store_window(
        &self,
        id: u64,
        window: WindowSession,
        workspace: Option<(PathBuf, WorkspaceSession)>,
        recent: Vec<PathBuf>,
        cx: &mut App,
    ) {
        let active = self.active_window_id(cx);
        self.session.update(cx, |store, _| {
            store.set_window(id, window);
            if let Some((path, state)) = workspace {
                store.set_workspace(path, state);
            }
            store.set_recent(recent);
            store.set_active(active);
            store.write();
        });
    }

    /// Drops a closed window from the session, so a relaunch does not reopen
    /// it. Only called for a user-requested close, not during app teardown.
    pub(crate) fn forget_window(&self, id: u64, cx: &mut App) {
        self.registry.update(cx, |registry, _| registry.remove(id));
        self.session.update(cx, |store, _| {
            store.remove_window(id);
            store.write();
        });
    }

    pub(crate) fn is_quitting(&self, cx: &App) -> bool {
        self.registry.read(cx).quitting
    }

    pub(crate) fn begin_quit(&self, cx: &mut App) {
        self.registry
            .update(cx, |registry, _| registry.quitting = true);
    }

    /// Number of *other* windows with unsaved edits. The caller reports its own
    /// window because reading the entity that is currently mid-update would
    /// borrow it twice.
    pub(crate) fn other_dirty_window_count(&self, except: u64, cx: &App) -> usize {
        self.registry
            .read(cx)
            .apps()
            .iter()
            .filter(|(id, _, app)| *id != except && app.read(cx).any_dirty())
            .count()
    }

    /// Captures every other window's frame and snapshot before the app quits.
    /// The window that started the quit saves itself directly.
    pub(crate) fn save_other_windows(&self, except: u64, cx: &mut App) {
        for (id, handle, app) in self.registry.read(cx).apps() {
            if id == except {
                continue;
            }
            handle
                .update(cx, |_root, window, cx| {
                    app.update(cx, |this, cx| {
                        this.remember_window_bounds(window);
                        this.save_session(cx);
                    });
                })
                .ok();
        }
    }

    /// Replaces the settings snapshot in every other window after one window
    /// writes them, so toggles and zoom apply everywhere.
    pub(crate) fn broadcast_settings(&self, source: u64, settings: &Settings, cx: &mut App) {
        let settings = settings.clone();
        for (id, handle, app) in self.registry.read(cx).apps() {
            if id == source {
                continue;
            }
            let settings = settings.clone();
            handle
                .update(cx, |_root, window, cx| {
                    app.update(cx, |this, cx| this.adopt_settings(settings, window, cx));
                })
                .ok();
        }
    }

    /// Rebuilds the app-level menu bar for every other window after the recent
    /// list changes.
    pub(crate) fn refresh_menus_others(&self, source: u64, cx: &mut App) {
        for (id, _handle, app) in self.registry.read(cx).apps() {
            if id == source {
                continue;
            }
            app.update(cx, |this, cx| this.refresh_menus(cx));
        }
    }

    /// Re-applies the theme in every window after the themes directory reloads.
    pub(crate) fn reapply_theme_all(&self, cx: &mut App) {
        for (_, _handle, app) in self.registry.read(cx).apps() {
            app.update(cx, |this, cx| this.reapply_theme(cx));
        }
    }

    /// Sends externally opened paths to the active window, or to the newest
    /// window when nothing is focused. With no windows open, this starts one.
    pub(crate) fn route_open(&self, paths: Vec<PathBuf>, cx: &mut App) {
        // A workspace file names a folder set, so it starts a window of its own
        // rather than opening as a text tab. Finder and `open -a` opens arrive
        // here too, not just the command line.
        if paths
            .iter()
            .any(|path| workspace_file::is_workspace_path(path))
        {
            let settings = load_settings().unwrap_or_default();
            self.open_window(cx, WindowSeed::for_paths(settings, paths));
            return;
        }
        if let Some((handle, app)) = self.target_window(cx) {
            handle
                .update(cx, |_root, window, cx| {
                    app.update(cx, |this, cx| {
                        this.request_open_many(paths, window, cx);
                    });
                })
                .ok();
        } else {
            let settings = load_settings().unwrap_or_default();
            self.open_window(cx, WindowSeed::for_paths(settings, paths));
        }
    }

    fn target_window(&self, cx: &App) -> Option<(AnyWindowHandle, Entity<TeakApp>)> {
        let apps = self.registry.read(cx).apps();
        let active = cx.active_window().map(|handle| handle.window_id());
        apps.iter()
            .find(|(_, handle, _)| Some(handle.window_id()) == active)
            .or_else(|| apps.last())
            .map(|(_, handle, app)| (*handle, app.clone()))
    }

    /// Opens a window from `seed` and registers it. Shared by `main`, the New
    /// Window command, and external file opens with no window available.
    pub(crate) fn open_window(
        &self,
        cx: &mut App,
        seed: WindowSeed,
    ) -> Option<(AnyWindowHandle, Entity<TeakApp>)> {
        open_app_window(cx, self, seed)
    }
}

/// What a new window starts with. `main` restores one per saved window; the New
/// Window command uses a blank seed; Open Folder in New Window passes roots.
pub(crate) struct WindowSeed {
    pub(crate) settings: Settings,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) open_paths: Vec<PathBuf>,
    pub(crate) session: Option<WindowSession>,
    pub(crate) frame: Option<WindowFrame>,
    /// The `.teak-workspace` this window shows, if any. Kept on the seed rather
    /// than reconstructed from the roots so the window knows which tab set to
    /// park when it switches away.
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) settings_error: Option<String>,
    /// Why a workspace file named on the command line could not be opened. The
    /// window still opens; `open_app_window` reports this once it has one.
    pub(crate) workspace_error: Option<String>,
}

impl WindowSeed {
    pub(crate) fn blank(settings: Settings) -> Self {
        Self {
            settings,
            roots: Vec::new(),
            open_paths: Vec::new(),
            session: None,
            frame: None,
            workspace: None,
            settings_error: None,
            workspace_error: None,
        }
    }

    /// Splits launch arguments into sidebar folders and files to open. Files
    /// that do not exist still go through `request_open`, which reports them.
    pub(crate) fn for_paths(settings: Settings, paths: Vec<PathBuf>) -> Self {
        let mut seed = Self::blank(settings);
        seed.add_paths(paths);
        seed
    }

    /// Adds paths a command-line launch asked for. A workspace file defines the
    /// folder set outright; a folder joins the set; a file opens ahead of
    /// whatever the session restores, in the order given, so the launch order
    /// wins over the saved tab order.
    pub(crate) fn add_paths(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            if workspace_file::is_workspace_path(&path) {
                self.open_workspace(&path);
            } else if path.is_dir() {
                // A restored window reads its roots back from the session, so
                // a folder has to land there rather than in `self.roots`.
                let roots = match self.session.as_mut() {
                    Some(session) => &mut session.roots,
                    None => &mut self.roots,
                };
                if !roots.contains(&path) {
                    roots.push(path);
                }
            } else {
                self.open_paths.push(path);
            }
        }
    }

    /// Binds the seed to a workspace file and replaces its folders with the
    /// ones the file declares. A bad or missing file leaves the folders and the
    /// binding alone and reports through `workspace_error`, so one unreadable
    /// argument cannot stop a window from opening.
    fn open_workspace(&mut self, path: &Path) {
        match workspace_file::load(path) {
            Ok(folders) => {
                self.workspace = Some(path.to_path_buf());
                self.workspace_error = None;
                self.set_roots(folders);
            }
            Err(message) => self.workspace_error = Some(message),
        }
    }

    /// Replaces the folder set wherever the seed keeps it: a restored window
    /// reads its roots back from its session, a blank one from `roots`.
    fn set_roots(&mut self, folders: Vec<PathBuf>) {
        match self.session.as_mut() {
            Some(session) => {
                session.workspace = self.workspace.clone();
                session.roots = folders;
            }
            None => self.roots = folders,
        }
    }

    /// Reopens a saved window with its workspace, tabs and frame. A zero-sized
    /// frame (written before the window was ever measured) is discarded so the
    /// platform picks a usable default.
    pub(crate) fn restored(settings: Settings, mut session: WindowSession) -> Self {
        let frame = session
            .frame
            .filter(|frame| frame.width > 0. && frame.height > 0.);
        session.frame = frame;
        Self {
            settings,
            roots: session.roots.clone(),
            open_paths: Vec::new(),
            workspace: session.workspace.clone(),
            frame,
            session: Some(session),
            settings_error: None,
            workspace_error: None,
        }
    }
}

/// Opens a frame derived from `window`, offset so a new window does not sit
/// exactly on top of the one that asked for it.
pub(crate) fn cascaded_frame(window: &Window) -> WindowFrame {
    let mut frame = frame_of(window.window_bounds());
    if frame.state == WindowFrameState::Windowed {
        frame.x += 24.;
        frame.y += 24.;
    } else {
        // A new window from a maximized or fullscreen one opens windowed, at
        // the restore size the platform reported.
        frame.state = WindowFrameState::Windowed;
    }
    frame
}

/// The frame of a live window, for the next session write.
pub(crate) fn frame_of_current(window: &Window) -> WindowFrame {
    frame_of(window.window_bounds())
}

fn frame_of(bounds: WindowBounds) -> WindowFrame {
    let (state, bounds) = match bounds {
        WindowBounds::Windowed(bounds) => (WindowFrameState::Windowed, bounds),
        WindowBounds::Maximized(bounds) => (WindowFrameState::Maximized, bounds),
        WindowBounds::Fullscreen(bounds) => (WindowFrameState::Fullscreen, bounds),
    };
    WindowFrame {
        x: bounds.origin.x.as_f32(),
        y: bounds.origin.y.as_f32(),
        width: bounds.size.width.as_f32(),
        height: bounds.size.height.as_f32(),
        state,
    }
}

fn bounds_of(frame: WindowFrame) -> WindowBounds {
    let bounds = Bounds::new(
        point(px(frame.x), px(frame.y)),
        size(px(frame.width), px(frame.height)),
    );
    match frame.state {
        WindowFrameState::Windowed => WindowBounds::Windowed(bounds),
        WindowFrameState::Maximized => WindowBounds::Maximized(bounds),
        WindowFrameState::Fullscreen => WindowBounds::Fullscreen(bounds),
    }
}

/// The one window bootstrap. `main` uses it for restored and launch-argument
/// windows; the app uses it for New Window and tab moves.
pub(crate) fn open_app_window(
    cx: &mut App,
    services: &AppServices,
    seed: WindowSeed,
) -> Option<(AnyWindowHandle, Entity<TeakApp>)> {
    let WindowSeed {
        settings,
        roots,
        open_paths,
        session,
        frame,
        workspace,
        settings_error,
        workspace_error,
    } = seed;
    let id = services
        .registry
        .update(cx, |registry, _| registry.allocate_id());
    let roots = session
        .as_ref()
        .map(|session| session.roots.clone())
        .unwrap_or(roots);
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(WINDOW_TITLE.into()),
            ..Default::default()
        }),
        window_bounds: frame.map(bounds_of),
        ..Default::default()
    };

    let services_for_window = services.clone();
    let result = gpui_kit::open_window(options, cx, move |window, cx| {
        let services = services_for_window.clone();
        // Apply the appearance through the new window so a window opened after
        // the themes directory reloaded picks up the current scheme too.
        crate::theme::apply(&settings, Some(window), cx);
        let app = cx.new(|cx| TeakApp::new(roots, workspace, settings, services, id, frame, cx));

        // Block the red close button (and app termination) while there are
        // unsaved edits, prompting instead.
        let close_guard = app.clone();
        window.on_window_should_close(cx, move |window, cx| {
            close_guard.update(cx, |this, cx| this.handle_window_should_close(window, cx))
        });

        // The notification layer is attached after this closure returns.
        if let Some(message) = settings_error {
            window.defer(cx, move |window, cx| {
                window.push_notification(Notification::error(message), cx);
            });
        }
        if let Some(message) = workspace_error {
            window.defer(cx, move |window, cx| {
                window.push_notification(Notification::error(message), cx);
            });
        }
        if let Some(session) = session {
            // The launch paths go through the restore so they land ahead of
            // the saved tabs, in argument order, and never fight the restore
            // over a file the session already had.
            app.update(cx, |this, cx| {
                this.restore_session(session, open_paths, window, cx)
            });
        } else {
            app.update(cx, |this, cx| {
                this.request_open_many(open_paths, window, cx)
            });
        }
        app.update(cx, |this, cx| this.check_font_family(window, cx));
        // Start following the workspace and reading git state without waiting
        // for a first activation event.
        app.update(cx, |this, cx| {
            this.start_watching(window, cx);
            this.refresh_git_status(cx);
        });

        let focus_handle = app.read(cx).focus_handle.clone();
        focus_handle.focus(window, cx);

        // Re-render whenever the window is (de)activated so a file deleted while
        // the app was in the background is reflected in its tab, and so the
        // app-level menu bar follows the window that is now active.
        app.update(cx, |_this, cx| {
            cx.observe_window_activation(window, |this, window, cx| {
                this.reload_tree(cx);
                this.remember_window_bounds(window);
                this.refresh_menus(cx);
                if window.is_window_active() {
                    this.mark_window_active(cx);
                    this.check_outside_changes(window, cx);
                }
                cx.notify()
            })
            .detach();
        });
        // Follow macOS appearance changes while the user chose System.
        app.update(cx, |_this, cx| {
            cx.observe_window_appearance(window, |this, window, cx| {
                this.on_system_appearance_changed(window, cx);
            })
            .detach();
        });

        app.update(cx, |this, cx| this.save_session(cx));
        app
    });

    match result {
        Ok((handle, app)) => {
            services.registry.update(cx, |registry, _| {
                registry.entries.push(WindowEntry {
                    id,
                    handle,
                    app: app.downgrade(),
                });
                registry.active = id;
            });
            Some((handle, app))
        }
        Err(error) => {
            eprintln!("could not open a window: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gpui_kit::{AppContext as _, Bounds, TestAppContext, WindowBounds, point, px, size};

    use super::{
        AppServices, RecentFiles, SessionStore, WindowFrame, WindowFrameState, WindowSeed,
        WindowSession, WorkspaceSession, bounds_of, frame_of, open_app_window,
    };
    use crate::settings::Settings;

    fn fixture(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("app-windows-{name}-{}", std::process::id()));
        std::fs::write(&path, "").unwrap();
        path
    }

    #[test]
    fn recent_seed_drops_missing_files() {
        let existing = fixture("recent");
        let missing = existing.with_extension("missing");
        let recent = RecentFiles::new(vec![missing, existing.clone()]);

        assert_eq!(recent.paths(), std::slice::from_ref(&existing));

        std::fs::remove_file(existing).unwrap();
    }

    #[test]
    fn session_store_keeps_one_entry_per_window() {
        let mut store = SessionStore::default();
        store.set_window(
            1,
            WindowSession {
                roots: vec![PathBuf::from("/one")],
                ..Default::default()
            },
        );
        store.set_window(
            2,
            WindowSession {
                roots: vec![PathBuf::from("/two")],
                ..Default::default()
            },
        );
        store.set_recent(vec![PathBuf::from("/one")]);

        let state = store.state();
        assert_eq!(state.windows.len(), 2);
        assert_eq!(state.recent, [PathBuf::from("/one")]);

        store.remove_window(1);
        assert_eq!(store.state().windows.len(), 1);
        assert_eq!(store.state().windows[0].roots, [PathBuf::from("/two")]);
    }

    #[test]
    fn launch_paths_join_the_target_window_after_its_session_tabs() {
        let dir = std::env::temp_dir().join(format!("teak-seed-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let saved = dir.join("saved.txt");
        let launched = dir.join("launched.txt");
        std::fs::write(&saved, "").unwrap();
        std::fs::write(&launched, "").unwrap();

        // A restored window keeps its saved folders and tabs; the launch
        // folder joins the roots and the launch file opens ahead of them.
        let mut restored = WindowSeed::restored(
            Settings::default(),
            WindowSession {
                roots: vec![dir.clone()],
                tabs: vec![saved.clone()],
                ..Default::default()
            },
        );
        restored.add_paths(vec![folder.clone(), launched.clone()]);
        let session = restored.session.as_ref().unwrap();
        assert_eq!(session.roots, vec![dir.clone(), folder.clone()]);
        assert_eq!(session.tabs, vec![saved]);
        assert_eq!(restored.open_paths, vec![launched.clone()]);

        // A blank window treats the launch paths like any other seed.
        let mut blank = WindowSeed::blank(Settings::default());
        blank.add_paths(vec![folder.clone(), launched.clone()]);
        assert_eq!(blank.roots, vec![folder]);
        assert_eq!(blank.open_paths, vec![launched]);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restored_window_drops_a_zero_sized_frame() {
        let seed = WindowSeed::restored(
            Settings::default(),
            WindowSession {
                frame: Some(WindowFrame::default()),
                ..Default::default()
            },
        );

        assert_eq!(seed.frame, None);
        assert_eq!(seed.session.and_then(|session| session.frame), None);
    }

    #[test]
    fn window_frames_round_trip_through_bounds() {
        let bounds = Bounds::new(point(px(40.), px(60.)), size(px(900.), px(700.)));
        for frame_state in [
            WindowFrameState::Windowed,
            WindowFrameState::Maximized,
            WindowFrameState::Fullscreen,
        ] {
            let frame = super::WindowFrame {
                x: 40.,
                y: 60.,
                width: 900.,
                height: 700.,
                state: frame_state,
            };
            let bounds = match frame_state {
                WindowFrameState::Windowed => WindowBounds::Windowed(bounds),
                WindowFrameState::Maximized => WindowBounds::Maximized(bounds),
                WindowFrameState::Fullscreen => WindowBounds::Fullscreen(bounds),
            };
            assert_eq!(frame_of(bounds), frame);
            assert_eq!(bounds_of(frame), bounds);
        }
    }

    #[test]
    fn a_workspace_file_seeds_its_folders_and_binding() {
        let dir = std::env::temp_dir().join(format!("teak-seed-workspace-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("app")).unwrap();
        std::fs::create_dir_all(dir.join("api")).unwrap();
        let file = dir.join("work.teak-workspace");
        std::fs::write(
            &file,
            r#"{ "folders": [{ "path": "app" }, { "path": "api" }, { "path": "app" }] }"#,
        )
        .unwrap();

        let seed = WindowSeed::for_paths(Settings::default(), vec![file.clone()]);

        assert_eq!(seed.workspace, Some(file));
        assert_eq!(seed.roots, [dir.join("app"), dir.join("api")]);
        assert!(seed.open_paths.is_empty());
        assert!(seed.workspace_error.is_none());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_broken_workspace_file_reports_and_leaves_the_window_usable() {
        let dir = std::env::temp_dir().join(format!("teak-seed-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("work.teak-workspace");
        std::fs::write(&file, r#"{ "folders": ["#).unwrap();

        let seed = WindowSeed::for_paths(Settings::default(), vec![file]);

        assert_eq!(seed.workspace, None, "a failed load does not bind");
        assert!(seed.roots.is_empty());
        assert!(
            seed.workspace_error
                .as_deref()
                .is_some_and(|error| error.starts_with("Could not parse workspace file")),
            "{:?}",
            seed.workspace_error
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_workspace_file_on_the_command_line_replaces_a_restored_folder_set() {
        let dir = std::env::temp_dir().join(format!("teak-seed-restored-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("saved")).unwrap();
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let file = dir.join("work.teak-workspace");
        std::fs::write(&file, r#"{ "folders": ["work"] }"#).unwrap();

        let mut seed = WindowSeed::restored(
            Settings::default(),
            WindowSession {
                roots: vec![dir.join("saved")],
                ..Default::default()
            },
        );
        seed.add_paths(vec![file.clone()]);

        let session = seed.session.as_ref().unwrap();
        assert_eq!(session.workspace, Some(file));
        assert_eq!(
            session.roots,
            [dir.join("work")],
            "the file wins over the session"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_workspace_parks_one_tab_set_per_file() {
        let mut store = SessionStore::default();
        let workspace = PathBuf::from("/w/work.teak-workspace");
        let parked = WorkspaceSession {
            tabs: vec![PathBuf::from("/w/work/a.rs")],
            active_tab: Some(PathBuf::from("/w/work/a.rs")),
            expanded: vec![PathBuf::from("/w/work")],
        };
        store.set_workspace(workspace.clone(), parked.clone());

        assert_eq!(
            store.state().workspaces.get(&workspace),
            Some(&parked),
            "the parked set is written with the window entries"
        );
        assert_eq!(store.parked(&workspace), Some(parked));
        assert_eq!(store.parked(Path::new("/w/other.teak-workspace")), None);
    }

    /// Closing one of two windows removes only it from the registry; closing
    /// the last one leaves the app running with no windows.
    #[gpui_kit::test]
    async fn closing_windows_updates_the_registry(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::keymap::bind_keys(cx);
        });
        let services = cx.update(|cx| AppServices::new(cx, Vec::new()));
        let (first, first_app) = cx
            .update(|cx| open_app_window(cx, &services, WindowSeed::blank(Settings::default())))
            .unwrap();
        let (second, second_app) = cx
            .update(|cx| open_app_window(cx, &services, WindowSeed::blank(Settings::default())))
            .unwrap();

        cx.update_window(second, |_, window, cx| {
            let closes =
                second_app.update(cx, |this, cx| this.handle_window_should_close(window, cx));
            assert!(closes, "a clean window closes without a prompt");
            window.remove_window();
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);
        assert!(cx.update(|cx| services.has_windows(cx)));

        cx.update_window(first, |_, window, cx| {
            let closes =
                first_app.update(cx, |this, cx| this.handle_window_should_close(window, cx));
            assert!(closes);
            window.remove_window();
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.windows().is_empty());
        assert!(!cx.update(|cx| services.has_windows(cx)));
    }
}
