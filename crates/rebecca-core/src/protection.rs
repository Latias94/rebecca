use std::path::{Path, PathBuf};

mod patterns;

use self::patterns::{
    NormalizedPath, contains_relative_control_segment, contains_traversal,
    is_allowlisted_maintenance_path, is_app_leftover_cache_path,
    is_regenerable_browser_cache_target_shape as is_regenerable_browser_cache_target_shape_impl,
    is_root, is_user_profile_root, is_windows_critical_path, looks_absolute_shape,
    normalize_raw_shape, normalize_shape_path, protected_category,
};

use crate::config::AppStorageEntry;
use crate::model::RuleTargetSpec;
use crate::path_overlap::paths_overlap;
use crate::safety_catalog::{SafetyCategory, SafetyKnowledge, default_safety_knowledge};

#[derive(Debug, Clone, Copy)]
pub struct ProtectionPolicy<'a> {
    safety_knowledge: &'a SafetyKnowledge,
    protected_storage: Option<&'a [AppStorageEntry]>,
    protected_paths: Option<&'a [PathBuf]>,
}

impl<'a> ProtectionPolicy<'a> {
    pub fn new() -> Self {
        Self {
            safety_knowledge: default_safety_knowledge(),
            protected_storage: None,
            protected_paths: None,
        }
    }

    pub fn with_safety_knowledge(mut self, safety_knowledge: &'a SafetyKnowledge) -> Self {
        self.safety_knowledge = safety_knowledge;
        self
    }

    pub fn with_protected_storage(mut self, protected_storage: &'a [AppStorageEntry]) -> Self {
        self.protected_storage = Some(protected_storage);
        self
    }

    pub fn with_protected_paths(mut self, protected_paths: &'a [PathBuf]) -> Self {
        self.protected_paths = Some(protected_paths);
        self
    }

    pub fn protected_storage(&self) -> Option<&'a [AppStorageEntry]> {
        self.protected_storage
    }

    pub fn protected_paths(&self) -> Option<&'a [PathBuf]> {
        self.protected_paths
    }

    pub fn safety_knowledge(&self) -> &'a SafetyKnowledge {
        self.safety_knowledge
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

        if let Some(protected_path) = self.protected_path_overlap(path) {
            return blocked(
                ProtectionBlockKind::UserProtectedPath,
                format!(
                    "target overlaps user-protected path at {}",
                    protected_path.display()
                ),
            );
        }

        if is_allowlisted_maintenance_path(&normalized, self.safety_knowledge) {
            return ProtectionAssessment::Allowed;
        }

        if is_windows_critical_path(&normalized.lower, self.safety_knowledge) {
            return blocked(
                ProtectionBlockKind::WindowsCriticalPath,
                "critical Windows path is protected".to_string(),
            );
        }

        if let Some(category) = protected_category(&normalized, self.safety_knowledge) {
            return blocked(
                ProtectionBlockKind::ProtectedCategory(category),
                format!("{} is protected", category.description()),
            );
        }

        ProtectionAssessment::Allowed
    }

    pub fn assess_app_leftover_path(&self, path: &Path) -> ProtectionAssessment {
        let normalized = NormalizedPath::new(path);
        let base_assessment = self.assess_path(path);

        match base_assessment {
            ProtectionAssessment::Allowed => {
                if is_app_leftover_cache_path(&normalized) {
                    ProtectionAssessment::Allowed
                } else {
                    blocked(
                        ProtectionBlockKind::ProtectedCategory(
                            ProtectedCategory::ApplicationDurableData,
                        ),
                        format!(
                            "app leftover target {} is not a recognized rebuildable cache path",
                            path.display()
                        ),
                    )
                }
            }
            ProtectionAssessment::Blocked(block)
                if matches!(
                    block.kind,
                    ProtectionBlockKind::ProtectedCategory(
                        ProtectedCategory::ApplicationDurableData
                    )
                ) && is_app_leftover_cache_path(&normalized) =>
            {
                ProtectionAssessment::Allowed
            }
            ProtectionAssessment::Blocked(block) => ProtectionAssessment::Blocked(block),
        }
    }

    pub fn assess_existing_app_leftover_path(&self, path: &Path) -> AppLeftoverPathDisposition {
        match self.assess_app_leftover_path(path) {
            ProtectionAssessment::Allowed => {}
            ProtectionAssessment::Blocked(block) => {
                return AppLeftoverPathDisposition::Blocked(block.message);
            }
        }

        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if crate::safety::is_reparse_like(&metadata) {
                    return AppLeftoverPathDisposition::Blocked(
                        "reparse-point traversal is disabled".to_string(),
                    );
                }

                AppLeftoverPathDisposition::Allowed
            }
            Err(_) => AppLeftoverPathDisposition::Missing,
        }
    }

    pub fn assess_relative_target_shape(&self, path: &Path) -> ProtectionAssessment {
        let normalized = normalize_shape_path(path);

        if normalized.is_empty() {
            return blocked(
                ProtectionBlockKind::EmptyPath,
                "empty relative target is not allowed".to_string(),
            );
        }

        if looks_absolute_shape(&normalized) {
            return blocked(
                ProtectionBlockKind::FilesystemRoot,
                "relative target must not be absolute".to_string(),
            );
        }

        if contains_relative_control_segment(&normalized) {
            return blocked(
                ProtectionBlockKind::PathTraversal,
                "relative target must not contain current or parent directory segments".to_string(),
            );
        }

        ProtectionAssessment::Allowed
    }

    pub fn assess_catalog_target_shape(&self, target: &RuleTargetSpec) -> ProtectionAssessment {
        match target {
            RuleTargetSpec::Template(template) | RuleTargetSpec::GlobTemplate(template) => {
                self.assess_path(Path::new(template.raw()))
            }
            RuleTargetSpec::ExactPath(path) => self.assess_path(path),
            RuleTargetSpec::SteamInstallTemplate(template) => self.assess_steam_catalog_shape(
                "Steam install",
                template.raw(),
                SafetyKnowledge::is_allowed_steam_install_target,
            ),
            RuleTargetSpec::SteamLibraryTemplate(template) => self.assess_steam_catalog_shape(
                "Steam library",
                template.raw(),
                SafetyKnowledge::is_allowed_steam_library_target,
            ),
        }
    }

    fn protected_storage_overlap(&self, path: &Path) -> Option<&'a AppStorageEntry> {
        self.protected_storage?
            .iter()
            .find(|entry| paths_overlap(path, &entry.path))
    }

    fn protected_path_overlap(&self, path: &Path) -> Option<&'a PathBuf> {
        self.protected_paths?
            .iter()
            .find(|protected_path| paths_overlap(path, protected_path))
    }

    fn assess_steam_catalog_shape(
        &self,
        scope: &'static str,
        raw: &str,
        is_allowlisted: fn(&SafetyKnowledge, &str) -> bool,
    ) -> ProtectionAssessment {
        let path = Path::new(raw);
        if let ProtectionAssessment::Blocked(block) = self.assess_relative_target_shape(path) {
            return ProtectionAssessment::Blocked(block);
        }

        let normalized = normalize_raw_shape(raw);
        if is_allowlisted(self.safety_knowledge, &normalized) {
            return ProtectionAssessment::Allowed;
        }

        blocked(
            ProtectionBlockKind::ProtectedCategory(ProtectedCategory::ApplicationDurableData),
            format!("{scope} target {raw} is not an allowlisted maintenance path"),
        )
    }
}

impl Default for ProtectionPolicy<'_> {
    fn default() -> Self {
        Self::new()
    }
}

pub fn is_regenerable_browser_cache_target_shape(spec: &RuleTargetSpec) -> bool {
    is_regenerable_browser_cache_target_shape_impl(spec)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLeftoverPathDisposition {
    Allowed,
    Missing,
    Blocked(String),
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
    UserProtectedPath,
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
            Self::UserProtectedPath => "user-protected-path",
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

impl From<SafetyCategory> for ProtectedCategory {
    fn from(category: SafetyCategory) -> Self {
        match category {
            SafetyCategory::Credentials => Self::Credentials,
            SafetyCategory::VpnProxyState => Self::VpnProxyState,
            SafetyCategory::AiToolDurableState => Self::AiToolDurableState,
            SafetyCategory::BrowserPrivateData => Self::BrowserPrivateData,
            SafetyCategory::CloudSyncedData => Self::CloudSyncedData,
            SafetyCategory::ContainerRuntimeState => Self::ContainerRuntimeState,
            SafetyCategory::StartupAutomation => Self::StartupAutomation,
            SafetyCategory::ApplicationDurableData => Self::ApplicationDurableData,
        }
    }
}

impl From<ProtectedCategory> for SafetyCategory {
    fn from(category: ProtectedCategory) -> Self {
        match category {
            ProtectedCategory::Credentials => Self::Credentials,
            ProtectedCategory::VpnProxyState => Self::VpnProxyState,
            ProtectedCategory::AiToolDurableState => Self::AiToolDurableState,
            ProtectedCategory::BrowserPrivateData => Self::BrowserPrivateData,
            ProtectedCategory::CloudSyncedData => Self::CloudSyncedData,
            ProtectedCategory::ContainerRuntimeState => Self::ContainerRuntimeState,
            ProtectedCategory::StartupAutomation => Self::StartupAutomation,
            ProtectedCategory::ApplicationDurableData => Self::ApplicationDurableData,
        }
    }
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
        default_safety_knowledge()
            .category_description(self.into())
            .unwrap_or(match self {
                Self::Credentials => "credential and password-manager data",
                Self::VpnProxyState => "VPN and proxy configuration",
                Self::AiToolDurableState => "AI and coding tool durable state",
                Self::BrowserPrivateData => "browser private data",
                Self::CloudSyncedData => "cloud-synced user data",
                Self::ContainerRuntimeState => "container and VM runtime state",
                Self::StartupAutomation => "startup automation",
                Self::ApplicationDurableData => "application durable data",
            })
    }
}

fn blocked(kind: ProtectionBlockKind, message: String) -> ProtectionAssessment {
    ProtectionAssessment::Blocked(ProtectionBlock { kind, message })
}
