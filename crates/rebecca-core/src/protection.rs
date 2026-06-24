use std::path::Path;

use crate::config::AppStorageEntry;
use crate::path_overlap::paths_overlap;

#[derive(Debug, Clone, Copy)]
pub struct ProtectionPolicy<'a> {
    protected_storage: Option<&'a [AppStorageEntry]>,
}

impl<'a> ProtectionPolicy<'a> {
    pub fn new() -> Self {
        Self {
            protected_storage: None,
        }
    }

    pub fn with_protected_storage(mut self, protected_storage: &'a [AppStorageEntry]) -> Self {
        self.protected_storage = Some(protected_storage);
        self
    }

    pub fn protected_storage(&self) -> Option<&'a [AppStorageEntry]> {
        self.protected_storage
    }

    pub fn assess_path(&self, path: &Path) -> ProtectionAssessment {
        let normalized = NormalizedPath::new(path);

        if normalized.raw.trim().is_empty() {
            return blocked(
                ProtectionBlockKind::EmptyPath,
                "empty path is not allowed".to_string(),
            );
        }

        if contains_traversal(&normalized.raw) {
            return blocked(
                ProtectionBlockKind::PathTraversal,
                "path traversal is not allowed".to_string(),
            );
        }

        if is_root(&normalized.raw) {
            return blocked(
                ProtectionBlockKind::FilesystemRoot,
                "filesystem roots are protected".to_string(),
            );
        }

        if is_windows_critical_path(&normalized.lower) {
            return blocked(
                ProtectionBlockKind::WindowsCriticalPath,
                "critical Windows path is protected".to_string(),
            );
        }

        if is_user_profile_root(&normalized.lower) {
            return blocked(
                ProtectionBlockKind::UserProfileRoot,
                "user profile root is protected".to_string(),
            );
        }

        if let Some(entry) = self.protected_storage_overlap(path) {
            return blocked(
                ProtectionBlockKind::RebeccaOwnedStorage,
                format!(
                    "target overlaps Rebecca-owned {} at {}",
                    entry.id.label(),
                    entry.path.display()
                ),
            );
        }

        if is_allowlisted_maintenance_path(&normalized) {
            return ProtectionAssessment::Allowed;
        }

        if let Some(category) = protected_category(&normalized) {
            return blocked(
                ProtectionBlockKind::ProtectedCategory(category),
                format!("{} is protected", category.description()),
            );
        }

        ProtectionAssessment::Allowed
    }

    fn protected_storage_overlap(&self, path: &Path) -> Option<&'a AppStorageEntry> {
        self.protected_storage?
            .iter()
            .find(|entry| paths_overlap(path, &entry.path))
    }
}

impl Default for ProtectionPolicy<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtectionAssessment {
    Allowed,
    Blocked(ProtectionBlock),
}

impl ProtectionAssessment {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionBlock {
    pub kind: ProtectionBlockKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtectionBlockKind {
    EmptyPath,
    PathTraversal,
    FilesystemRoot,
    WindowsCriticalPath,
    UserProfileRoot,
    RebeccaOwnedStorage,
    ProtectedCategory(ProtectedCategory),
}

impl ProtectionBlockKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::EmptyPath => "empty-path",
            Self::PathTraversal => "path-traversal",
            Self::FilesystemRoot => "filesystem-root",
            Self::WindowsCriticalPath => "windows-critical-path",
            Self::UserProfileRoot => "user-profile-root",
            Self::RebeccaOwnedStorage => "rebecca-owned-storage",
            Self::ProtectedCategory(category) => category.label(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProtectedCategory {
    Credentials,
    VpnProxyState,
    AiToolDurableState,
    BrowserPrivateData,
    CloudSyncedData,
    ContainerRuntimeState,
    StartupAutomation,
    ApplicationDurableData,
}

impl ProtectedCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Credentials => "credentials",
            Self::VpnProxyState => "vpn-proxy-state",
            Self::AiToolDurableState => "ai-tool-durable-state",
            Self::BrowserPrivateData => "browser-private-data",
            Self::CloudSyncedData => "cloud-synced-data",
            Self::ContainerRuntimeState => "container-runtime-state",
            Self::StartupAutomation => "startup-automation",
            Self::ApplicationDurableData => "application-durable-data",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Credentials => "credential and password-manager data",
            Self::VpnProxyState => "VPN and proxy configuration",
            Self::AiToolDurableState => "AI and coding tool durable state",
            Self::BrowserPrivateData => "browser private data",
            Self::CloudSyncedData => "cloud-synced user data",
            Self::ContainerRuntimeState => "container and VM runtime state",
            Self::StartupAutomation => "startup automation",
            Self::ApplicationDurableData => "application durable data",
        }
    }
}

fn blocked(kind: ProtectionBlockKind, message: String) -> ProtectionAssessment {
    ProtectionAssessment::Blocked(ProtectionBlock { kind, message })
}

struct NormalizedPath {
    raw: String,
    lower: String,
}

impl NormalizedPath {
    fn new(path: &Path) -> Self {
        let raw =
            trim_trailing_separators(path.as_os_str().to_string_lossy().replace('\\', "/").trim());
        let lower = raw.to_ascii_lowercase();

        Self { raw, lower }
    }

    fn segments(&self) -> Vec<&str> {
        self.lower
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect()
    }
}

fn trim_trailing_separators(path: &str) -> String {
    let mut normalized = path.to_string();

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    normalized
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

    if let Some(rest) = normalized.strip_prefix("//") {
        return rest
            .split('/')
            .filter(|segment| !segment.is_empty())
            .count()
            == 2;
    }

    false
}

fn is_windows_critical_path(lower: &str) -> bool {
    const PROTECTED_PREFIXES: &[&str] = &[
        "c:/windows",
        "c:/program files",
        "c:/program files (x86)",
        "c:/programdata",
        "c:/$recycle.bin",
        "c:/recovery",
        "c:/system volume information",
    ];

    PROTECTED_PREFIXES
        .iter()
        .any(|prefix| same_or_descendant(lower, prefix))
}

fn is_user_profile_root(lower: &str) -> bool {
    let mut parts = lower.split('/').filter(|segment| !segment.is_empty());
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(drive), Some("users"), Some(_name), None) if drive.ends_with(':')
    )
}

fn is_allowlisted_maintenance_path(path: &NormalizedPath) -> bool {
    let segments = path.segments();

    is_chromium_cache_path(&segments)
        || is_firefox_cache_path(&segments)
        || is_electron_cache_path(&segments)
        || is_jetbrains_cache_path(&segments)
        || is_cargo_cache_path(&segments)
        || is_pip_cache_path(&segments)
        || is_npm_cache_path(&segments)
        || is_known_temp_or_report_path(&segments)
}

fn protected_category(path: &NormalizedPath) -> Option<ProtectedCategory> {
    let segments = path.segments();

    if is_credential_path(&segments) {
        return Some(ProtectedCategory::Credentials);
    }

    if is_vpn_or_proxy_path(&segments) {
        return Some(ProtectedCategory::VpnProxyState);
    }

    if is_ai_tool_durable_state_path(&segments) {
        return Some(ProtectedCategory::AiToolDurableState);
    }

    if is_browser_private_data_path(&segments) {
        return Some(ProtectedCategory::BrowserPrivateData);
    }

    if is_cloud_synced_path(&segments) {
        return Some(ProtectedCategory::CloudSyncedData);
    }

    if is_container_runtime_path(&segments) {
        return Some(ProtectedCategory::ContainerRuntimeState);
    }

    if is_startup_automation_path(&segments) {
        return Some(ProtectedCategory::StartupAutomation);
    }

    if is_application_durable_data_path(&segments) {
        return Some(ProtectedCategory::ApplicationDurableData);
    }

    None
}

fn is_chromium_cache_path(segments: &[&str]) -> bool {
    find_segment(segments, "user data")
        .is_some_and(|index| chromium_cache_tail_is_allowed(segments, index + 1))
        || find_segment(segments, "htmlcache")
            .is_some_and(|index| chromium_cache_tail_is_allowed(segments, index + 1))
}

fn is_firefox_cache_path(segments: &[&str]) -> bool {
    find_sequence(segments, &["mozilla", "firefox", "profiles"]).is_some_and(|index| {
        segments
            .get(index + 4)
            .is_some_and(|segment| matches!(*segment, "cache2" | "startupcache"))
    })
}

fn is_electron_cache_path(segments: &[&str]) -> bool {
    let Some(appdata_index) = find_segment(segments, "appdata") else {
        return false;
    };

    let app = segments.get(appdata_index + 2).copied().unwrap_or_default();
    let cache = segments.get(appdata_index + 3).copied().unwrap_or_default();

    matches!(app, "code" | "discord" | "discordptb" | "discordcanary")
        && matches!(cache, "cache" | "code cache" | "gpucache" | "cacheddata")
}

fn is_jetbrains_cache_path(segments: &[&str]) -> bool {
    find_segment(segments, "jetbrains").is_some_and(|index| {
        segments
            .get(index + 2)
            .is_some_and(|segment| *segment == "caches")
    })
}

fn is_cargo_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["registry", "cache"])
        || has_sequence(segments, &["registry", "index"])
        || has_sequence(segments, &["registry", "src"])
        || has_sequence(segments, &["git", "db"])
        || has_sequence(segments, &["git", "checkouts"])
}

fn is_pip_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["pip", "cache"])
}

fn is_npm_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["npm-cache", "_cacache"])
}

fn is_known_temp_or_report_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["appdata", "local", "temp"])
        || has_sequence(segments, &["microsoft", "windows", "wer", "reportarchive"])
        || has_sequence(segments, &["microsoft", "windows", "wer", "reportqueue"])
}

fn is_browser_cache_segment(segment: &str) -> bool {
    matches!(segment, "cache" | "code cache" | "gpucache")
}

fn chromium_cache_tail_is_allowed(segments: &[&str], start: usize) -> bool {
    matches!(
        (segments.get(start), segments.get(start + 1)),
        (Some(&"default"), Some(cache)) if is_browser_cache_segment(cache)
    ) || matches!(
        (segments.get(start), segments.get(start + 1)),
        (Some(profile), Some(cache))
            if profile.starts_with("profile ") && is_browser_cache_segment(cache)
    )
}

fn is_credential_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["microsoft", "credentials"])
        || has_sequence(segments, &["microsoft", "protect"])
        || has_sequence(segments, &["microsoft", "crypto"])
        || has_sequence(segments, &["microsoft", "vault"])
        || has_any_segment(segments, &[".ssh", ".gnupg", "1password", "bitwarden"])
        || segments
            .last()
            .is_some_and(|leaf| matches!(*leaf, "credentials.toml" | "key4.db" | "logins.json"))
}

fn is_vpn_or_proxy_path(segments: &[&str]) -> bool {
    has_any_segment(
        segments,
        &[
            "clash",
            "clash verge",
            "tailscale",
            "wireguard",
            "v2ray",
            "shadowsocks",
            "nekoray",
            "sing-box",
        ],
    )
}

fn is_ai_tool_durable_state_path(segments: &[&str]) -> bool {
    has_any_segment(segments, &[".codex", ".claude", ".cursor", ".ollama"])
        || has_any_segment(segments, &["claude", "cursor", "ollama", "chatgpt"])
        || has_sequence(segments, &["code", "user"])
}

fn is_browser_private_data_path(segments: &[&str]) -> bool {
    if find_segment(segments, "user data").is_some_and(|index| {
        segments.get(index + 2).is_some_and(|segment| {
            matches!(
                *segment,
                "history"
                    | "cookies"
                    | "login data"
                    | "web data"
                    | "local storage"
                    | "indexeddb"
                    | "service worker"
                    | "network"
            )
        })
    }) {
        return true;
    }

    find_sequence(segments, &["mozilla", "firefox", "profiles"]).is_some_and(|index| {
        segments.get(index + 4).is_some_and(|segment| {
            matches!(
                *segment,
                "cookies.sqlite"
                    | "places.sqlite"
                    | "logins.json"
                    | "key4.db"
                    | "formhistory.sqlite"
                    | "storage"
            )
        })
    })
}

fn is_cloud_synced_path(segments: &[&str]) -> bool {
    has_any_segment(
        segments,
        &[
            "onedrive",
            "icloud drive",
            "icloud photos",
            "dropbox",
            "google drive",
            "box",
            "mega",
        ],
    )
}

fn is_container_runtime_path(segments: &[&str]) -> bool {
    has_any_segment(
        segments,
        &[
            ".docker",
            ".podman",
            ".kube",
            ".wslconfig",
            "docker",
            "docker desktop",
            "podman",
            "rancher desktop",
            "orbstack",
        ],
    )
}

fn is_startup_automation_path(segments: &[&str]) -> bool {
    has_sequence(
        segments,
        &["microsoft", "windows", "start menu", "programs", "startup"],
    )
}

fn is_application_durable_data_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["steam", "userdata"])
        || has_sequence(segments, &["steamapps", "common"])
        || has_sequence(segments, &["steamapps", "workshop"])
        || has_sequence(segments, &["steamapps", "compatdata"])
        || has_any_segment(
            segments,
            &["local storage", "indexeddb", "service worker", "network"],
        )
}

fn same_or_descendant(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn has_sequence(segments: &[&str], sequence: &[&str]) -> bool {
    find_sequence(segments, sequence).is_some()
}

fn find_sequence(segments: &[&str], sequence: &[&str]) -> Option<usize> {
    if sequence.is_empty() || segments.len() < sequence.len() {
        return None;
    }

    segments
        .windows(sequence.len())
        .position(|window| window == sequence)
}

fn find_segment(segments: &[&str], needle: &str) -> Option<usize> {
    segments.iter().position(|segment| *segment == needle)
}

fn has_any_segment(segments: &[&str], needles: &[&str]) -> bool {
    segments
        .iter()
        .any(|segment| needles.iter().any(|needle| segment == needle))
}
