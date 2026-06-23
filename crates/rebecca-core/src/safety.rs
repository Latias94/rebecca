use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathDisposition {
    Allowed,
    Skipped(String),
    Blocked(String),
}

impl PathDisposition {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

pub fn assess_path(path: &Path) -> PathDisposition {
    let normalized = normalize(path);

    if normalized.trim().is_empty() {
        return PathDisposition::Blocked("empty path".to_string());
    }

    if contains_traversal(&normalized) {
        return PathDisposition::Blocked("path traversal is not allowed".to_string());
    }

    if is_root(&normalized) {
        return PathDisposition::Blocked("filesystem roots are protected".to_string());
    }

    let lower = normalized.to_ascii_lowercase();

    if is_windows_critical_path(&lower) {
        return PathDisposition::Blocked("critical Windows path is protected".to_string());
    }

    if is_user_profile_root(&lower) {
        return PathDisposition::Blocked("user profile root is protected".to_string());
    }

    PathDisposition::Allowed
}

pub fn assess_existing_path(path: &Path) -> PathDisposition {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_reparse_like(&metadata) {
                return PathDisposition::Blocked("reparse-point traversal is disabled".to_string());
            }

            assess_path(path)
        }
        Err(_) => PathDisposition::Skipped("path does not exist".to_string()),
    }
}

pub fn is_reparse_like(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
            || metadata.file_type().is_symlink()
    }

    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn normalize(path: &Path) -> String {
    path.as_os_str()
        .to_string_lossy()
        .replace('\\', "/")
        .trim()
        .to_string()
}

fn contains_traversal(normalized: &str) -> bool {
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .any(|segment| segment == "..")
}

fn is_root(normalized: &str) -> bool {
    if normalized == "/" || normalized == "//" {
        return true;
    }

    if normalized.len() == 2 {
        let bytes = normalized.as_bytes();
        return bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    }

    if normalized.len() == 3 {
        let bytes = normalized.as_bytes();
        return bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/';
    }

    false
}

fn is_windows_critical_path(lower: &str) -> bool {
    let protected_prefixes = [
        "c:/windows",
        "c:/program files",
        "c:/program files (x86)",
        "c:/programdata",
        "c:/$recycle.bin",
    ];

    protected_prefixes.iter().any(|prefix| {
        lower == *prefix
            || lower
                .strip_prefix(prefix)
                .map(|suffix| suffix.starts_with('/'))
                .unwrap_or(false)
    })
}

fn is_user_profile_root(lower: &str) -> bool {
    let mut parts = lower.split('/').filter(|segment| !segment.is_empty());
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(drive), Some("users"), Some(_name), None) if drive.ends_with(':')
    )
}
