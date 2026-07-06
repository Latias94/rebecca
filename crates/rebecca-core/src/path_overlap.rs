use std::path::{Component, Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathRelation {
    Same,
    Ancestor,
    Descendant,
    Unrelated,
}

pub(crate) fn paths_overlap(left: &Path, right: &Path) -> bool {
    !matches!(path_relation(left, right), PathRelation::Unrelated)
}

pub(crate) fn path_is_same_or_child(parent: &Path, child: &Path) -> bool {
    matches!(
        path_relation(child, parent),
        PathRelation::Same | PathRelation::Descendant
    )
}

pub(crate) fn path_relation(left: &Path, right: &Path) -> PathRelation {
    let left = comparable_components(left);
    let right = comparable_components(right);

    if left.is_empty() || right.is_empty() {
        return PathRelation::Unrelated;
    }

    if left == right {
        return PathRelation::Same;
    }

    if right.len() > left.len() && right.starts_with(&left) {
        return PathRelation::Ancestor;
    }

    if left.len() > right.len() && left.starts_with(&right) {
        return PathRelation::Descendant;
    }

    PathRelation::Unrelated
}

fn comparable_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
            Component::RootDir => Some(std::path::MAIN_SEPARATOR.to_string()),
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_string()),
            Component::CurDir => None,
        })
        .map(|component| {
            if cfg!(windows) {
                component.to_ascii_lowercase()
            } else {
                component
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::path::Path;

    use super::{PathRelation, path_relation, paths_overlap};

    #[test]
    fn detects_same_child_and_parent_paths() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Rebecca");
        let child = root.join("cache").join("scan");

        assert!(paths_overlap(&root, &root));
        assert!(paths_overlap(&root, &child));
        assert!(paths_overlap(&child, &root));
        assert!(!paths_overlap(&root, &temp.path().join("Other")));
    }

    #[test]
    fn reports_directional_path_relation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let child = root.join("cache");

        assert_eq!(path_relation(&root, &root), PathRelation::Same);
        assert_eq!(path_relation(&root, &child), PathRelation::Ancestor);
        assert_eq!(path_relation(&child, &root), PathRelation::Descendant);
        assert_eq!(
            path_relation(&root, &temp.path().join("other")),
            PathRelation::Unrelated
        );
    }

    #[cfg(windows)]
    #[test]
    fn compares_windows_paths_case_insensitively() {
        assert!(paths_overlap(
            Path::new(r"C:\Users\Alice\AppData\Local\Rebecca"),
            Path::new(r"c:\users\alice\appdata\local\rebecca\cache"),
        ));
    }
}
