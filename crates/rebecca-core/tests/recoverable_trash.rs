use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use rebecca_core::executor::{CleanupBackend, RecoverableTrashAdapter, RecoverableTrashBackend};
use rebecca_core::plan::{CleanupTarget, CleanupTargetDeletionStyle};
use rebecca_core::{DeleteMode, Result};

#[derive(Debug, Clone)]
struct DirectoryMoveTrashAdapter {
    trash_dir: PathBuf,
}

impl RecoverableTrashAdapter for DirectoryMoveTrashAdapter {
    fn delete_paths(&self, paths: &[PathBuf]) -> Result<()> {
        fs::create_dir_all(&self.trash_dir)?;
        for path in paths {
            if matches!(path.try_exists(), Ok(false)) {
                continue;
            }
            let destination = unique_trash_destination(&self.trash_dir, path);
            fs::rename(path, destination)?;
        }
        Ok(())
    }
}

fn recoverable_backend(temp: &tempfile::TempDir) -> RecoverableTrashBackend {
    RecoverableTrashBackend::with_adapter(DirectoryMoveTrashAdapter {
        trash_dir: temp.path().join("recoverable-trash"),
    })
}

fn unique_trash_destination(trash_dir: &Path, path: &Path) -> PathBuf {
    let base_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| OsString::from("entry"));
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

#[test]
fn recoverable_trash_backend_moves_file_when_supported() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let file = temp.path().join("delete-me.tmp");
    fs::write(&file, b"trash")?;

    let target = CleanupTarget::allowed(
        "test.user-temp",
        file.clone(),
        5,
        DeleteMode::RecoverableDelete,
    );
    let backend = recoverable_backend(&temp);
    let outcome = backend.delete(&target)?;

    assert_eq!(outcome.pending_reclaim_bytes, 5);
    assert_eq!(outcome.note.as_deref(), Some("moved to recoverable trash"));
    assert!(!file.exists());
    Ok(())
}

#[test]
fn recoverable_trash_backend_preserves_target_directory() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let cache = temp.path().join("cache");
    let child = cache.join("entry.tmp");
    fs::create_dir_all(&cache)?;
    fs::write(&child, b"trash")?;

    let target = CleanupTarget::allowed(
        "test.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecoverableDelete,
    );
    let backend = recoverable_backend(&temp);
    let outcome = backend.delete(&target)?;

    assert_eq!(outcome.pending_reclaim_bytes, 5);
    assert!(cache.exists());
    assert!(!child.exists());
    Ok(())
}

#[test]
fn recoverable_trash_backend_preserves_target_directory_after_batching_multiple_entries()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let cache = temp.path().join("cache");
    fs::create_dir_all(cache.join("nested"))?;
    fs::write(cache.join("entry-a.tmp"), b"trash")?;
    fs::write(cache.join("entry-b.tmp"), b"trash")?;
    fs::write(cache.join("nested").join("entry-c.tmp"), b"trash")?;

    let target = CleanupTarget::allowed(
        "test.user-temp",
        cache.clone(),
        15,
        DeleteMode::RecoverableDelete,
    );
    let backend = recoverable_backend(&temp);
    let outcome = backend.delete(&target)?;

    assert_eq!(outcome.pending_reclaim_bytes, 15);
    assert!(cache.exists());
    assert_eq!(fs::read_dir(&cache)?.count(), 0);
    Ok(())
}

#[cfg(any(unix, windows))]
#[test]
fn recoverable_trash_backend_refuses_preserve_root_reparse_children() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside");
    let normal_child = cache.join("entry.tmp");
    let linked_child = cache.join("linked");
    fs::create_dir_all(&cache)?;
    fs::create_dir_all(&outside)?;
    fs::write(&normal_child, b"trash")?;

    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &linked_child)?;

    #[cfg(windows)]
    {
        if let Err(err) = std::os::windows::fs::symlink_dir(&outside, &linked_child) {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                return Ok(());
            }
            return Err(err.into());
        }
    }

    let target = CleanupTarget::allowed(
        "test.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecoverableDelete,
    );
    let backend = recoverable_backend(&temp);
    let err = backend.delete(&target).unwrap_err();

    assert!(err.to_string().contains("refused reparse child"));
    assert!(cache.exists());
    assert!(normal_child.exists());
    assert!(linked_child.exists());
    Ok(())
}

#[test]
fn recoverable_trash_backend_batch_deletes_multiple_targets() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let first = temp.path().join("first.tmp");
    let second = temp.path().join("second.tmp");
    fs::write(&first, b"trash")?;
    fs::write(&second, b"trash")?;

    let first_target = CleanupTarget::allowed(
        "test.first",
        first.clone(),
        5,
        DeleteMode::RecoverableDelete,
    );
    let second_target = CleanupTarget::allowed(
        "test.second",
        second.clone(),
        7,
        DeleteMode::RecoverableDelete,
    );
    let backend = recoverable_backend(&temp);
    let outcomes = backend.delete_batch(&[&first_target, &second_target]);

    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].as_ref().unwrap().pending_reclaim_bytes, 5);
    assert_eq!(outcomes[1].as_ref().unwrap().pending_reclaim_bytes, 7);
    assert!(!first.exists());
    assert!(!second.exists());
    Ok(())
}

#[test]
fn recoverable_trash_backend_batch_preserves_directory_roots() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let cache = temp.path().join("cache");
    let child = cache.join("entry.tmp");
    fs::create_dir_all(&cache)?;
    fs::write(&child, b"trash")?;

    let target = CleanupTarget::allowed(
        "test.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecoverableDelete,
    );
    let backend = recoverable_backend(&temp);
    let outcomes = backend.delete_batch(&[&target]);

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].as_ref().unwrap().pending_reclaim_bytes, 5);
    assert!(cache.exists());
    assert!(!child.exists());
    Ok(())
}

#[test]
fn recoverable_trash_backend_deletes_whole_path_when_requested() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let cache = temp.path().join("cache");
    let child = cache.join("entry.tmp");
    fs::create_dir_all(&cache)?;
    fs::write(&child, b"trash")?;

    let target = CleanupTarget::allowed(
        "test.user-temp",
        cache.clone(),
        5,
        DeleteMode::RecoverableDelete,
    )
    .with_deletion_style(CleanupTargetDeletionStyle::DeleteWholePath);
    let backend = recoverable_backend(&temp);
    let outcome = backend.delete(&target)?;

    assert_eq!(outcome.pending_reclaim_bytes, 5);
    assert!(!cache.exists());
    assert!(!child.exists());
    Ok(())
}
