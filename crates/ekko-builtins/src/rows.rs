//! Sidebar row model and session navigation order: builds the flattened row
//! list (project rule rows + indented session rows) from the grouped session
//! list, and provides the pure navigation helpers (next/prev session,
//! next/prev project) shared by the sidebar surface and the keybinding
//! handlers. Pure policy — this is exactly the kind of ordering decision
//! that must live in a builtin, not the core.

use ekko_ext::{ProjectGroup, SessionState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarRowKind {
    Project(usize),
    Session {
        project_index: usize,
        session_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidebarRow {
    pub kind: SidebarRowKind,
    pub text: String,
    /// The attached/current session (drawn without a bg highlight, bright fg).
    pub current: bool,
    pub alive: bool,
}

/// Build the flattened sidebar row list: one `Project` row per group followed
/// by its `Session` rows.
pub fn build_rows(projects: &[ProjectGroup], current_session: &str) -> Vec<SidebarRow> {
    let mut rows = Vec::new();
    for (project_index, project) in projects.iter().enumerate() {
        rows.push(SidebarRow {
            kind: SidebarRowKind::Project(project_index),
            text: project.name.clone(),
            current: false,
            alive: project
                .sessions
                .iter()
                .any(|s| s.state == SessionState::Alive),
        });
        for (session_index, session) in project.sessions.iter().enumerate() {
            rows.push(SidebarRow {
                kind: SidebarRowKind::Session {
                    project_index,
                    session_index,
                },
                text: session.name.clone(),
                current: session.name == current_session,
                alive: session.state == SessionState::Alive,
            });
        }
    }
    rows
}

/// Clamp/adjust `scroll` so `selected` stays within the visible window.
pub fn ensure_visible(
    scroll: usize,
    selected: usize,
    visible_rows: usize,
    row_count: usize,
) -> usize {
    if visible_rows == 0 || row_count == 0 {
        return 0;
    }
    let max_scroll = row_count.saturating_sub(visible_rows);
    let mut scroll = scroll.min(max_scroll);
    if selected < scroll {
        scroll = selected;
    } else if selected >= scroll + visible_rows {
        scroll = selected + 1 - visible_rows;
    }
    scroll.min(max_scroll)
}

/// All session names in sidebar order (project by project, session by session).
fn flatten_session_names(projects: &[ProjectGroup]) -> Vec<&str> {
    projects
        .iter()
        .flat_map(|p| p.sessions.iter().map(|s| s.name.as_str()))
        .collect()
}

/// The session immediately after `current` in sidebar order, wrapping around.
pub fn next_session_name(projects: &[ProjectGroup], current: &str) -> Option<String> {
    let names = flatten_session_names(projects);
    step_session_name(&names, current, 1)
}

/// The session immediately before `current` in sidebar order, wrapping around.
pub fn prev_session_name(projects: &[ProjectGroup], current: &str) -> Option<String> {
    let names = flatten_session_names(projects);
    step_session_name(&names, current, -1)
}

fn step_session_name(names: &[&str], current: &str, delta: i32) -> Option<String> {
    if names.len() < 2 {
        return None;
    }
    let len = names.len() as i32;
    let index = names.iter().position(|n| *n == current).unwrap_or(0) as i32;
    let next = (((index + delta) % len) + len) % len;
    Some(names[next as usize].to_string())
}

fn project_index_of(projects: &[ProjectGroup], current: &str) -> Option<usize> {
    projects
        .iter()
        .position(|p| p.sessions.iter().any(|s| s.name == current))
}

/// The first session of the next/previous project (wrapping), or `None` if
/// there is only one project (or none).
pub fn adjacent_project_first_session(
    projects: &[ProjectGroup],
    current: &str,
    forward: bool,
) -> Option<String> {
    if projects.len() < 2 {
        return None;
    }
    let current_index = project_index_of(projects, current)?;
    let len = projects.len() as i32;
    let delta = if forward { 1 } else { -1 };
    let mut index = current_index as i32;
    for _ in 0..projects.len() {
        index = (((index + delta) % len) + len) % len;
        if let Some(first) = projects[index as usize].sessions.first() {
            return Some(first.name.clone());
        }
        if index as usize == current_index {
            break;
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn test_group(name: &str, session_names: &[&str]) -> ProjectGroup {
    use ekko_ext::SessionEntry;
    use std::path::PathBuf;
    ProjectGroup {
        name: name.to_string(),
        sessions: session_names
            .iter()
            .map(|n| SessionEntry {
                name: n.to_string(),
                cwd: PathBuf::from("/tmp"),
                state: SessionState::Alive,
                created_at_secs: 0,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, session_names: &[&str]) -> ProjectGroup {
        test_group(name, session_names)
    }

    #[test]
    fn build_rows_produces_project_then_sessions() {
        let projects = vec![group("mux", &["a", "b"])];
        let rows = build_rows(&projects, "a");
        assert_eq!(rows.len(), 3);
        assert!(matches!(rows[0].kind, SidebarRowKind::Project(0)));
        assert!(matches!(rows[1].kind, SidebarRowKind::Session { .. }));
        assert!(rows[1].current);
        assert!(!rows[2].current);
    }

    #[test]
    fn next_and_prev_session_wrap_within_and_across_projects() {
        let projects = vec![group("a", &["s1", "s2"]), group("b", &["s3"])];
        assert_eq!(next_session_name(&projects, "s1"), Some("s2".to_string()));
        assert_eq!(next_session_name(&projects, "s2"), Some("s3".to_string()));
        assert_eq!(next_session_name(&projects, "s3"), Some("s1".to_string()));
        assert_eq!(prev_session_name(&projects, "s1"), Some("s3".to_string()));
    }

    #[test]
    fn next_session_is_none_when_only_one_session_exists() {
        let projects = vec![group("a", &["only"])];
        assert_eq!(next_session_name(&projects, "only"), None);
    }

    #[test]
    fn adjacent_project_jumps_to_first_session() {
        let projects = vec![
            group("a", &["s1", "s2"]),
            group("b", &["s3"]),
            group("c", &["s4"]),
        ];
        assert_eq!(
            adjacent_project_first_session(&projects, "s1", true),
            Some("s3".to_string())
        );
        assert_eq!(
            adjacent_project_first_session(&projects, "s3", false),
            Some("s1".to_string())
        );
        assert_eq!(
            adjacent_project_first_session(&projects, "s4", true),
            Some("s1".to_string())
        );
    }

    #[test]
    fn adjacent_project_none_with_single_project() {
        let projects = vec![group("a", &["s1"])];
        assert_eq!(adjacent_project_first_session(&projects, "s1", true), None);
    }

    #[test]
    fn ensure_visible_scrolls_minimally() {
        assert_eq!(ensure_visible(0, 5, 3, 10), 3);
        assert_eq!(ensure_visible(3, 1, 3, 10), 1);
        assert_eq!(ensure_visible(0, 2, 3, 10), 0);
    }
}
