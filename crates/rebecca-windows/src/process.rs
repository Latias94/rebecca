use rebecca_core::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveProcess {
    pub process_id: u32,
    pub executable_name: String,
}

pub fn active_processes() -> Result<Vec<ActiveProcess>> {
    platform::active_processes()
}

#[cfg(windows)]
mod platform {
    use super::ActiveProcess;
    use rebecca_core::error::{RebeccaError, Result};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    pub fn active_processes() -> Result<Vec<ActiveProcess>> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|err| RebeccaError::ApplicationDiscoveryFailed(err.to_string()))?;
        let snapshot = SnapshotHandle(snapshot);

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..PROCESSENTRY32W::default()
        };
        let mut processes = Vec::new();

        if unsafe { Process32FirstW(snapshot.0, &mut entry) }.is_err() {
            return Ok(processes);
        }

        loop {
            if let Some(executable_name) = process_name(&entry) {
                processes.push(ActiveProcess {
                    process_id: entry.th32ProcessID,
                    executable_name,
                });
            }

            if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
                break;
            }
        }

        Ok(processes)
    }

    struct SnapshotHandle(HANDLE);

    impl Drop for SnapshotHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    fn process_name(entry: &PROCESSENTRY32W) -> Option<String> {
        let end = entry
            .szExeFile
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(entry.szExeFile.len());
        if end == 0 {
            return None;
        }

        Some(String::from_utf16_lossy(&entry.szExeFile[..end]))
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ActiveProcess;
    use rebecca_core::error::{RebeccaError, Result};

    pub fn active_processes() -> Result<Vec<ActiveProcess>> {
        Err(RebeccaError::PlatformUnavailable(
            "Windows process diagnostics are not available on this platform".to_string(),
        ))
    }
}
