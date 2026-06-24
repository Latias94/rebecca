use std::path::{Component, Path};

pub(crate) fn paths_overlap(left: &Path, right: &Path) -> bool {
    same_or_child_path(left, right) || same_or_child_path(right, left)
}

fn same_or_child_path(parent: &Path, child: &Path) -> bool {
    let parent = comparable_components(parent);
    let child = comparable_components(child);
    !parent.is_empty() && child.len() >= parent.len() && child.starts_with(&parent)
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
    use std::path::Path;

    use super::paths_overlap;

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

    #[cfg(windows)]
    #[test]
    fn compares_windows_paths_case_insensitively() {
        assert!(paths_overlap(
            Path::new(r"C:\Users\Alice\AppData\Local\Rebecca"),
            Path::new(r"c:\users\alice\appdata\local\rebecca\cache"),
        ));
    }
}
