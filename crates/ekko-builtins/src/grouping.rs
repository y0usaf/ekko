//! The stock session-grouping policy: group by the parent directory of each
//! session's cwd ("project"), sorted by name.

use std::sync::Arc;

use anyhow::Result;
use ekko_ext::{
    Extension, ExtensionHost, ExtensionManifest, ProjectGroup, SessionEntry, SessionGrouperSpec,
};

pub struct GroupingExtension;

impl Extension for GroupingExtension {
    fn manifest(&self) -> ExtensionManifest {
        ExtensionManifest {
            id: "ekko-builtins.grouping".into(),
            name: "project grouping".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "group sessions by parent directory of cwd".into(),
        }
    }

    fn register(&self, host: &mut dyn ExtensionHost) -> Result<()> {
        host.register_session_grouper(SessionGrouperSpec {
            name: "parent-dir".into(),
            group: Arc::new(group_by_project),
        })
    }
}

/// Group sessions by the parent directory of their `cwd`. Groups are sorted
/// by name; sessions within a group are sorted by name.
pub fn group_by_project(mut sessions: Vec<SessionEntry>) -> Vec<ProjectGroup> {
    sessions.sort_by(|a, b| a.name.cmp(&b.name));

    let mut groups: Vec<ProjectGroup> = Vec::new();
    for session in sessions {
        let project_name = project_name_for(&session.cwd);
        if let Some(group) = groups.iter_mut().find(|g| g.name == project_name) {
            group.sessions.push(session);
        } else {
            groups.push(ProjectGroup {
                name: project_name,
                sessions: vec![session],
            });
        }
    }
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    groups
}

fn project_name_for(cwd: &std::path::Path) -> String {
    let parent = cwd.parent().unwrap_or(cwd);
    parent
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| parent.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ekko_ext::SessionState;
    use std::path::PathBuf;

    fn entry(name: &str, cwd: &str) -> SessionEntry {
        SessionEntry {
            name: name.to_string(),
            cwd: PathBuf::from(cwd),
            state: SessionState::Alive,
            created_at_secs: 0,
        }
    }

    #[test]
    fn groups_sessions_sharing_a_parent_directory() {
        let sessions = vec![
            entry("b", "/home/user/dev/mux"),
            entry("a", "/home/user/dev/mux"),
            entry("c", "/home/user/other/proj"),
        ];
        let groups = group_by_project(sessions);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "dev");
        assert_eq!(
            groups[0]
                .sessions
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(groups[1].name, "other");
        assert_eq!(groups[1].sessions[0].name, "c");
    }

    #[test]
    fn project_name_falls_back_to_full_path_at_root() {
        assert_eq!(project_name_for(std::path::Path::new("/")), "/");
    }

    #[test]
    fn groups_are_sorted_by_name() {
        let sessions = vec![entry("x", "/z/proj"), entry("y", "/a/proj")];
        let groups = group_by_project(sessions);
        assert_eq!(groups[0].name, "a");
        assert_eq!(groups[1].name, "z");
    }
}
