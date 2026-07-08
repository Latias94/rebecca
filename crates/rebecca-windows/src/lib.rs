pub mod apps;
pub mod process;
pub mod recycle_bin;
pub mod steam;
pub mod usn_cache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeLevel {
    StandardUser,
    Elevated,
    Unknown,
}

pub fn current_privilege_level() -> PrivilegeLevel {
    platform::current_privilege_level()
}

#[cfg(windows)]
mod platform {
    use windows::Win32::UI::Shell::IsUserAnAdmin;

    pub fn current_privilege_level() -> super::PrivilegeLevel {
        unsafe {
            if IsUserAnAdmin().as_bool() {
                super::PrivilegeLevel::Elevated
            } else {
                super::PrivilegeLevel::StandardUser
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn current_privilege_level() -> super::PrivilegeLevel {
        super::PrivilegeLevel::Unknown
    }
}
