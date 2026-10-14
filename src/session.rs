use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths::support_dir;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SessionState {
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) tabs: Vec<PathBuf>,
    pub(crate) active_tab: Option<PathBuf>,
    pub(crate) expanded: Vec<PathBuf>,
    pub(crate) selected_path: Option<PathBuf>,
    pub(crate) sidebar_visible: bool,
    /// Sidebar width in pixels, as last dragged by the user.
    pub(crate) sidebar_width: Option<u32>,
}

pub(crate) fn session_path() -> Option<PathBuf> {
    Some(support_dir()?.join("session.json"))
}

pub(crate) fn load_session() -> Option<SessionState> {
    let path = session_path()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_round_trips_through_json() {
        let state = SessionState {
            roots: vec![PathBuf::from("/tmp/project"), PathBuf::from("/tmp/other")],
            tabs: vec![PathBuf::from("/tmp/project/a.rs")],
            active_tab: Some(PathBuf::from("/tmp/project/a.rs")),
            expanded: vec![PathBuf::from("/tmp/project")],
            selected_path: None,
            sidebar_visible: true,
            sidebar_width: Some(260),
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: SessionState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.roots, state.roots);
        assert_eq!(restored.tabs, state.tabs);
        assert_eq!(restored.active_tab, state.active_tab);
        assert_eq!(restored.expanded, state.expanded);
        assert_eq!(restored.selected_path, state.selected_path);
        assert!(restored.sidebar_visible);
        assert_eq!(restored.sidebar_width, Some(260));
    }

    #[test]
    fn session_defaults_missing_fields() {
        let restored: SessionState = serde_json::from_str(r#"{"tabs": ["/tmp/a.txt"]}"#).unwrap();

        assert_eq!(restored.tabs, [PathBuf::from("/tmp/a.txt")]);
        assert!(restored.roots.is_empty());
        assert!(!restored.sidebar_visible);
    }

    #[test]
    fn session_ignores_the_legacy_workspace_root_key() {
        let restored: SessionState =
            serde_json::from_str(r#"{"workspace_root": "/tmp/project"}"#).unwrap();

        assert!(restored.roots.is_empty());
    }
}
