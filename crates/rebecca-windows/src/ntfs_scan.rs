use std::path::Path;

use rebecca_core::RebeccaError;

pub const EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL: &str = "windows-ntfs-mft-experimental";

pub fn live_volume_index_unavailable(path: &Path) -> RebeccaError {
    RebeccaError::PlatformUnavailable(format!(
        "{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL} is read-only and experimental; live NTFS volume indexing is not enabled for {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::{EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL, live_volume_index_unavailable};

    #[test]
    fn unavailable_error_names_experimental_backend() {
        let err = live_volume_index_unavailable(std::path::Path::new("C:\\Temp"));

        assert!(
            err.to_string()
                .contains(EXPERIMENTAL_NTFS_MFT_BACKEND_LABEL)
        );
    }
}
