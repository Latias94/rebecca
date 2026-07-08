#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecycleBinState {
    pub root: Option<String>,
    pub bytes: u64,
    pub items: u64,
}

pub fn query_recycle_bin(root: Option<&str>) -> Result<RecycleBinState, String> {
    platform::query_recycle_bin(root)
}

pub fn empty_recycle_bin(root: Option<&str>) -> Result<RecycleBinState, String> {
    platform::empty_recycle_bin(root)
}

#[cfg(windows)]
mod platform {
    use super::RecycleBinState;
    use windows::Win32::UI::Shell::{
        SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW, SHQUERYRBINFO,
        SHQueryRecycleBinW,
    };
    use windows::core::PCWSTR;

    pub fn query_recycle_bin(root: Option<&str>) -> Result<RecycleBinState, String> {
        let root = normalize_root(root)?;
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            ..SHQUERYRBINFO::default()
        };
        let wide_root = root.as_ref().map(|root| encode_wide(root));
        let root_ptr = wide_root
            .as_ref()
            .map_or(PCWSTR::null(), |root| PCWSTR(root.as_ptr()));

        // SAFETY: `root_ptr` is either null for all recycle bins or points to a
        // nul-terminated UTF-16 buffer that lives until the call returns.
        unsafe {
            SHQueryRecycleBinW(root_ptr, &mut info)
                .map_err(|err| format!("failed to query Recycle Bin: {err}"))?;
        }

        Ok(RecycleBinState {
            root,
            bytes: info.i64Size.max(0) as u64,
            items: info.i64NumItems.max(0) as u64,
        })
    }

    pub fn empty_recycle_bin(root: Option<&str>) -> Result<RecycleBinState, String> {
        let before = query_recycle_bin(root)?;
        let wide_root = before.root.as_ref().map(|root| encode_wide(root));
        let root_ptr = wide_root
            .as_ref()
            .map_or(PCWSTR::null(), |root| PCWSTR(root.as_ptr()));
        let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;

        // SAFETY: `root_ptr` follows the same lifetime and nul-termination rules
        // as the query path above. We pass no window handle because this is a CLI.
        unsafe {
            SHEmptyRecycleBinW(None, root_ptr, flags)
                .map_err(|err| format!("failed to empty Recycle Bin: {err}"))?;
        }

        Ok(before)
    }

    fn normalize_root(root: Option<&str>) -> Result<Option<String>, String> {
        let Some(root) = root else {
            return Ok(None);
        };
        let trimmed = root.trim();
        if trimmed.is_empty() {
            return Err("drive cannot be empty".to_string());
        }

        let mut chars = trimmed.chars();
        let Some(letter) = chars.next() else {
            return Err("drive cannot be empty".to_string());
        };
        if !letter.is_ascii_alphabetic() {
            return Err(format!("drive must start with a letter, got {trimmed}"));
        }
        let rest = chars.as_str();
        if !matches!(rest, "" | ":" | ":\\" | ":/") {
            return Err(format!("drive must look like C or C:, got {trimmed}"));
        }

        Ok(Some(format!("{}:\\", letter.to_ascii_uppercase())))
    }

    fn encode_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod platform {
    use super::RecycleBinState;

    pub fn query_recycle_bin(_root: Option<&str>) -> Result<RecycleBinState, String> {
        Err("Recycle Bin management is only available on Windows".to_string())
    }

    pub fn empty_recycle_bin(_root: Option<&str>) -> Result<RecycleBinState, String> {
        Err("Recycle Bin management is only available on Windows".to_string())
    }
}
