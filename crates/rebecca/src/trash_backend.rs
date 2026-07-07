use rebecca::core::executor::RecoverableTrashBackend;

pub(crate) fn recoverable_trash_backend() -> RecoverableTrashBackend {
    #[cfg(debug_assertions)]
    {
        if let Some(trash_dir) = test_recoverable_trash_dir() {
            return RecoverableTrashBackend::with_adapter(DirectoryMoveTrashAdapter { trash_dir });
        }
    }

    RecoverableTrashBackend::new()
}

#[cfg(debug_assertions)]
#[derive(Debug, Clone)]
struct DirectoryMoveTrashAdapter {
    trash_dir: std::path::PathBuf,
}

#[cfg(debug_assertions)]
impl rebecca::core::executor::RecoverableTrashAdapter for DirectoryMoveTrashAdapter {
    fn delete_paths(&self, paths: &[std::path::PathBuf]) -> rebecca::core::Result<()> {
        std::fs::create_dir_all(&self.trash_dir)?;
        for path in paths {
            if matches!(path.try_exists(), Ok(false)) {
                continue;
            }
            let destination = unique_trash_destination(&self.trash_dir, path);
            std::fs::rename(path, destination)?;
        }
        Ok(())
    }
}

#[cfg(debug_assertions)]
fn test_recoverable_trash_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("REBECCA_TEST_RECOVERABLE_TRASH_DIR")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
}

#[cfg(debug_assertions)]
fn unique_trash_destination(
    trash_dir: &std::path::Path,
    path: &std::path::Path,
) -> std::path::PathBuf {
    let base_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("entry"));
    let process_id = std::process::id();

    for index in 0.. {
        let mut name = base_name.clone();
        name.push(format!(".{process_id}.{index}"));
        let destination = trash_dir.join(name);
        if !destination.exists() {
            return destination;
        }
    }

    unreachable!("unbounded unique trash destination search should return")
}
