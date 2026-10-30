use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths::support_dir;
use crate::search::SearchOptions;

/// Everything the app persists between launches: one entry per open window,
/// the tab sets parked by workspaces, and the shared recent lists.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SessionState {
    pub(crate) windows: Vec<WindowSession>,
    /// The tabs each workspace reopens with, keyed by the workspace file's
    /// path. A window showing a workspace publishes here as well as in
    /// `windows`, so switching away parks the set and switching back restores
    /// it even when the window is gone.
    pub(crate) workspaces: BTreeMap<PathBuf, WorkspaceSession>,
    /// Recently opened files and folders, newest first, for File > Open Recent.
    /// Shared by every window, so it lives outside [`WindowSession`]. Workspace
    /// files share the list, and opening one is routed to the workspace instead
    /// of to a tab.
    pub(crate) recent: Vec<PathBuf>,
}

/// One window's workspace and open tabs.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WindowSession {
    /// The `.teak-workspace` this window is showing, if any. Its folders are in
    /// `roots` as well; the file is remembered so a relaunch can keep the
    /// binding and a switch can park the tabs under the right key.
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) tabs: Vec<PathBuf>,
    pub(crate) active_tab: Option<PathBuf>,
    pub(crate) expanded: Vec<PathBuf>,
    pub(crate) selected_path: Option<PathBuf>,
    pub(crate) sidebar_visible: bool,
    /// Sidebar width in pixels, as last dragged by the user.
    pub(crate) sidebar_width: Option<u32>,
    /// Which view the sidebar showed.
    pub(crate) sidebar_view: SidebarView,
    /// The Find panel's last query, Where field and toggles.
    pub(crate) find_query: String,
    pub(crate) find_where: String,
    pub(crate) find_replace: String,
    pub(crate) find_options: SearchOptions,
    /// The window's frame at the last save, so a relaunch can reopen it where
    /// the user left it.
    pub(crate) frame: Option<WindowFrame>,
}

/// The tabs a workspace reopens with. Parked while another workspace is
/// showing, and written by whichever window is showing it, so two windows on
/// one workspace resolve the same way settings and the recent list do: the last
/// writer wins.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceSession {
    pub(crate) tabs: Vec<PathBuf>,
    pub(crate) active_tab: Option<PathBuf>,
    pub(crate) expanded: Vec<PathBuf>,
}

/// What the sidebar shows: the folder tree, or the files git reports changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SidebarView {
    #[default]
    Explorer,
    Changes,
}

/// A serializable window frame. GPUI's `WindowBounds` is not serde-friendly, so
/// the values are stored flat and converted at the window boundary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WindowFrame {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) state: WindowFrameState,
}

impl Default for WindowFrame {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            width: 0.,
            height: 0.,
            state: WindowFrameState::Windowed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WindowFrameState {
    #[default]
    Windowed,
    Maximized,
    Fullscreen,
}

/// The pre-multi-window session shape: one window's fields at the top level.
/// Only used to migrate an existing `session.json`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LegacySession {
    roots: Vec<PathBuf>,
    tabs: Vec<PathBuf>,
    active_tab: Option<PathBuf>,
    expanded: Vec<PathBuf>,
    selected_path: Option<PathBuf>,
    sidebar_visible: bool,
    sidebar_width: Option<u32>,
    recent: Vec<PathBuf>,
    find_query: String,
    find_where: String,
    find_replace: String,
    find_options: SearchOptions,
}

impl LegacySession {
    fn into_session(self) -> SessionState {
        SessionState {
            windows: vec![WindowSession {
                workspace: None,
                roots: self.roots,
                tabs: self.tabs,
                active_tab: self.active_tab,
                expanded: self.expanded,
                selected_path: self.selected_path,
                sidebar_visible: self.sidebar_visible,
                sidebar_width: self.sidebar_width,
                sidebar_view: SidebarView::Explorer,
                find_query: self.find_query,
                find_where: self.find_where,
                find_replace: self.find_replace,
                find_options: self.find_options,
                frame: None,
            }],
            workspaces: BTreeMap::new(),
            recent: self.recent,
        }
    }
}

/// How many entries File > Open Recent keeps. Long enough that the Open Recent
/// picker is worth searching: a filter over a handful of rows is decoration.
pub(crate) const MAX_RECENT: usize = 50;

/// Moves `path` to the front of `recent`, dropping its older entry and any
/// beyond [`MAX_RECENT`].
pub(crate) fn push_recent(recent: &mut Vec<PathBuf>, path: PathBuf) {
    recent.retain(|existing| *existing != path);
    recent.insert(0, path);
    recent.truncate(MAX_RECENT);
}

pub(crate) fn session_path() -> Option<PathBuf> {
    Some(support_dir()?.join("session.json"))
}

pub(crate) fn load_session() -> Option<SessionState> {
    parse_session(&std::fs::read_to_string(session_path()?).ok()?)
}

/// Parses either the current session shape or the older flat one. The check for
/// a `windows` key keeps the two apart: an old file has no such key, and the
/// new shape's `#[serde(default)]` would otherwise swallow it as empty.
pub(crate) fn parse_session(data: &str) -> Option<SessionState> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    if value.get("windows").is_some() {
        serde_json::from_value(value).ok()
    } else {
        serde_json::from_value::<LegacySession>(value)
            .ok()
            .map(LegacySession::into_session)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn window(path: &str) -> WindowSession {
        WindowSession {
            workspace: Some(PathBuf::from(format!("{path}/work.teak-workspace"))),
            roots: vec![PathBuf::from(path)],
            tabs: vec![PathBuf::from(format!("{path}/a.rs"))],
            active_tab: Some(PathBuf::from(format!("{path}/a.rs"))),
            expanded: vec![PathBuf::from(path)],
            selected_path: None,
            sidebar_visible: true,
            sidebar_width: Some(260),
            sidebar_view: SidebarView::Changes,
            find_query: "needle".into(),
            find_where: "-vendor/**".into(),
            find_replace: "pin".into(),
            find_options: SearchOptions {
                regex: true,
                ..Default::default()
            },
            frame: Some(WindowFrame {
                x: 40.,
                y: 60.,
                width: 900.,
                height: 700.,
                state: WindowFrameState::Windowed,
            }),
        }
    }

    #[test]
    fn session_round_trips_through_json() {
        let state = SessionState {
            windows: vec![window("/tmp/project"), window("/tmp/other")],
            workspaces: BTreeMap::from([(
                PathBuf::from("/tmp/project/work.teak-workspace"),
                WorkspaceSession {
                    tabs: vec![PathBuf::from("/tmp/project/a.rs")],
                    active_tab: Some(PathBuf::from("/tmp/project/a.rs")),
                    expanded: vec![PathBuf::from("/tmp/project")],
                },
            )]),
            recent: vec![PathBuf::from("/tmp/project/a.rs")],
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.windows.len(), 2);
        assert_eq!(restored.windows[0].workspace, state.windows[0].workspace);
        assert_eq!(restored.windows[0].roots, state.windows[0].roots);
        assert_eq!(restored.windows[0].tabs, state.windows[0].tabs);
        assert_eq!(restored.windows[0].active_tab, state.windows[0].active_tab);
        assert_eq!(restored.windows[0].expanded, state.windows[0].expanded);
        assert_eq!(restored.windows[0].selected_path, None);
        assert!(restored.windows[0].sidebar_visible);
        assert_eq!(restored.windows[0].sidebar_width, Some(260));
        assert_eq!(restored.windows[0].sidebar_view, SidebarView::Changes);
        assert_eq!(restored.windows[0].find_query, "needle");
        assert_eq!(restored.windows[0].find_where, "-vendor/**");
        assert_eq!(restored.windows[0].find_replace, "pin");
        assert_eq!(
            restored.windows[0].find_options,
            state.windows[0].find_options
        );
        assert_eq!(restored.windows[0].frame, state.windows[0].frame);
        assert_eq!(restored.workspaces, state.workspaces);
        assert_eq!(restored.recent, state.recent);
    }

    #[test]
    fn a_workspace_file_binding_survives_a_missing_workspaces_table() {
        let restored = parse_session(
            r#"{
                "windows": [{
                    "workspace": "/tmp/project/work.teak-workspace",
                    "roots": ["/tmp/project"],
                    "tabs": ["/tmp/project/a.txt"]
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(
            restored.windows[0].workspace.as_deref(),
            Some(Path::new("/tmp/project/work.teak-workspace"))
        );
        assert!(restored.workspaces.is_empty());
    }

    #[test]
    fn legacy_flat_session_becomes_one_window() {
        let restored = parse_session(
            r#"{
                "roots": ["/tmp/project"],
                "tabs": ["/tmp/project/a.txt"],
                "active_tab": "/tmp/project/a.txt",
                "expanded": ["/tmp/project"],
                "sidebar_visible": true,
                "sidebar_width": 210,
                "recent": ["/tmp/project/a.txt"],
                "find_query": "needle"
            }"#,
        )
        .unwrap();

        assert_eq!(restored.windows.len(), 1);
        assert_eq!(restored.windows[0].roots, [PathBuf::from("/tmp/project")]);
        assert_eq!(
            restored.windows[0].tabs,
            [PathBuf::from("/tmp/project/a.txt")]
        );
        assert_eq!(restored.windows[0].sidebar_width, Some(210));
        assert_eq!(restored.windows[0].find_query, "needle");
        assert_eq!(restored.recent, [PathBuf::from("/tmp/project/a.txt")]);
    }

    #[test]
    fn push_recent_moves_to_front_without_duplicates() {
        let mut recent = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        push_recent(&mut recent, PathBuf::from("/b"));
        assert_eq!(recent, [PathBuf::from("/b"), PathBuf::from("/a")]);

        for ix in 0..MAX_RECENT + 5 {
            push_recent(&mut recent, PathBuf::from(format!("/f{ix}")));
        }
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent[0], PathBuf::from(format!("/f{}", MAX_RECENT + 4)));
    }

    #[test]
    fn session_defaults_missing_fields() {
        let restored = parse_session(r#"{"windows": [{"tabs": ["/tmp/a.txt"]}]}"#).unwrap();

        assert_eq!(restored.windows.len(), 1);
        assert_eq!(restored.windows[0].tabs, [PathBuf::from("/tmp/a.txt")]);
        assert!(restored.windows[0].roots.is_empty());
        assert!(!restored.windows[0].sidebar_visible);
        assert!(restored.recent.is_empty());
    }

    #[test]
    fn session_ignores_the_legacy_workspace_root_key() {
        let restored = parse_session(r#"{"workspace_root": "/tmp/project"}"#).unwrap();

        assert_eq!(restored.windows.len(), 1);
        assert!(restored.windows[0].roots.is_empty());
    }

    #[test]
    fn empty_windows_array_stays_empty() {
        let restored = parse_session(r#"{"windows": [], "recent": []}"#).unwrap();

        assert!(restored.windows.is_empty());
    }
}
