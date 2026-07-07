use std::path::Path;

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
    let raw = path.as_os_str().to_string_lossy();
    let windows_like = cfg!(windows) || raw.contains('\\') || has_drive_prefix(&raw);
    let normalized = raw.replace('\\', "/");
    let mut cursor = normalized.as_str();
    let mut components = Vec::new();

    if has_drive_prefix(cursor) {
        components.push(cursor[..2].to_string());
        cursor = &cursor[2..];
    }

    if cursor.starts_with('/') {
        components.push("/".to_string());
        cursor = cursor.trim_start_matches('/');
    }

    components.extend(cursor.split('/').filter_map(|component| match component {
        "" | "." => None,
        value => Some(value.to_string()),
    }));

    if windows_like {
        components
            .into_iter()
            .map(|component| component.to_ascii_lowercase())
            .collect()
    } else {
        components
    }
}

fn has_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn compares_windows_like_paths_case_insensitively() {
        assert!(paths_overlap(
            Path::new(r"C:\Users\Alice\AppData\Local\Rebecca"),
            Path::new(r"c:\users\alice\appdata\local\rebecca\cache"),
        ));
    }
}
