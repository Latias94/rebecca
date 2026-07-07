use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::{Platform, RebeccaError, Result};

pub const SAFETY_CATALOG_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyCategory {
    Credentials,
    VpnProxyState,
    AiToolDurableState,
    BrowserPrivateData,
    CloudSyncedData,
    ContainerRuntimeState,
    StartupAutomation,
    ApplicationDurableData,
}

impl SafetyCategory {
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SafetyCatalogFile {
    catalog_version: u16,
    default_platform: Platform,
    #[serde(default)]
    warning_kinds: Vec<WarningKindDefinition>,
    #[serde(default)]
    protected_categories: Vec<ProtectedCategoryDefinition>,
    #[serde(default)]
    maintenance_allowlist: SafetyPatternSetFile,
    #[serde(default)]
    protected_patterns: Vec<ProtectedPatternDefinition>,
    #[serde(default)]
    platforms: Vec<PlatformSafetyDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformSafetyDefinition {
    platform: Platform,
    #[serde(default)]
    critical_path_prefixes: Vec<String>,
    #[serde(default)]
    maintenance_allowlist: SafetyPatternSetFile,
    #[serde(default)]
    protected_patterns: Vec<ProtectedPatternDefinition>,
    #[serde(default)]
    steam_install_allowlist: Vec<String>,
    #[serde(default)]
    steam_library_allowlist: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WarningKindDefinition {
    id: String,
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedCategoryDefinition {
    id: SafetyCategory,
    description: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SafetyPatternSetFile {
    #[serde(default)]
    segments: Vec<String>,
    #[serde(default)]
    sequences: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedPatternDefinition {
    category: SafetyCategory,
    #[serde(default)]
    segments: Vec<String>,
    #[serde(default)]
    leaf_names: Vec<String>,
    #[serde(default)]
    sequences: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct SafetyKnowledge {
    platform: Platform,
    warning_kinds: Vec<WarningKind>,
    categories: Vec<ProtectedCategoryKnowledge>,
    critical_path_prefixes: Vec<String>,
    maintenance_allowlist: SafetyPatternSet,
    protected_patterns: Vec<ProtectedPattern>,
    steam_install_allowlist: Vec<String>,
    steam_library_allowlist: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SafetyCatalog {
    default_platform: Platform,
    platform_knowledge: Vec<SafetyKnowledge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WarningKind {
    id: String,
    description: String,
}

impl WarningKind {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProtectedCategoryKnowledge {
    id: SafetyCategory,
    description: String,
}

impl ProtectedCategoryKnowledge {
    pub fn id(&self) -> SafetyCategory {
        self.id
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

#[derive(Debug, Clone, Default)]
pub struct SafetyPatternSet {
    segment_matches: Vec<String>,
    sequence_matches: Vec<Vec<String>>,
}

impl SafetyPatternSet {
    pub fn matches(&self, segments: &[&str]) -> bool {
        self.has_any_segment(segments) || self.has_any_sequence(segments)
    }

    fn merged(&self, other: Self) -> Self {
        let mut segment_matches = self.segment_matches.clone();
        segment_matches.extend(other.segment_matches);
        segment_matches.sort();
        segment_matches.dedup();

        let mut sequence_matches = self.sequence_matches.clone();
        sequence_matches.extend(other.sequence_matches);
        sequence_matches.sort();
        sequence_matches.dedup();

        Self {
            segment_matches,
            sequence_matches,
        }
    }

    fn has_any_segment(&self, segments: &[&str]) -> bool {
        segments
            .iter()
            .any(|segment| self.segment_matches.iter().any(|needle| segment == needle))
    }

    fn has_any_sequence(&self, segments: &[&str]) -> bool {
        self.sequence_matches
            .iter()
            .any(|sequence| has_sequence(segments, sequence))
    }
}

#[derive(Debug, Clone)]
pub struct ProtectedPattern {
    category: SafetyCategory,
    segment_matches: Vec<String>,
    leaf_names: Vec<String>,
    sequence_matches: Vec<Vec<String>>,
}

impl ProtectedPattern {
    pub fn category(&self) -> SafetyCategory {
        self.category
    }

    pub fn matches(&self, segments: &[&str]) -> bool {
        self.matches_segment(segments)
            || self.matches_leaf(segments)
            || self.matches_sequence(segments)
    }

    fn matches_segment(&self, segments: &[&str]) -> bool {
        segments
            .iter()
            .any(|segment| self.segment_matches.iter().any(|needle| segment == needle))
    }

    fn matches_leaf(&self, segments: &[&str]) -> bool {
        segments
            .last()
            .is_some_and(|leaf| self.leaf_names.iter().any(|needle| leaf == needle))
    }

    fn matches_sequence(&self, segments: &[&str]) -> bool {
        self.sequence_matches
            .iter()
            .any(|sequence| has_sequence(segments, sequence))
    }
}

impl SafetyKnowledge {
    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn warning_kinds(&self) -> &[WarningKind] {
        &self.warning_kinds
    }

    pub fn categories(&self) -> &[ProtectedCategoryKnowledge] {
        &self.categories
    }

    pub fn critical_path_prefixes(&self) -> &[String] {
        &self.critical_path_prefixes
    }

    pub fn maintenance_allowlist(&self) -> &SafetyPatternSet {
        &self.maintenance_allowlist
    }

    pub fn protected_patterns(&self) -> &[ProtectedPattern] {
        &self.protected_patterns
    }

    pub fn category_description(&self, category: SafetyCategory) -> Option<&str> {
        self.categories
            .iter()
            .find(|entry| entry.id == category)
            .map(ProtectedCategoryKnowledge::description)
    }

    pub fn is_allowed_steam_install_target(&self, normalized: &str) -> bool {
        self.steam_install_allowlist
            .iter()
            .any(|target| target == normalized)
    }

    pub fn is_allowed_steam_library_target(&self, normalized: &str) -> bool {
        self.steam_library_allowlist
            .iter()
            .any(|target| target == normalized)
    }
}

impl SafetyCatalog {
    pub fn default_platform(&self) -> Platform {
        self.default_platform
    }

    pub fn platform_knowledge(&self) -> &[SafetyKnowledge] {
        &self.platform_knowledge
    }

    pub fn default_knowledge(&self) -> Option<&SafetyKnowledge> {
        self.knowledge_for_platform(self.default_platform)
    }

    pub fn knowledge_for_platform(&self, platform: Platform) -> Option<&SafetyKnowledge> {
        self.platform_knowledge
            .iter()
            .find(|knowledge| knowledge.platform == platform)
    }
}

pub fn parse_safety_catalog(path: &str, raw: &str) -> Result<SafetyCatalog> {
    let catalog = toml::from_str::<SafetyCatalogFile>(raw).map_err(|err| {
        RebeccaError::SafetyCatalogInvalid(format!("{path} is invalid safety catalog data: {err}"))
    })?;
    compile_safety_catalog(path, catalog)
}

pub fn parse_safety_catalog_file(path: &str, raw: &str) -> Result<SafetyKnowledge> {
    let catalog = parse_safety_catalog(path, raw)?;
    catalog.default_knowledge().cloned().ok_or_else(|| {
        RebeccaError::SafetyCatalogInvalid(format!(
            "{path} default platform {} has no platform safety block",
            catalog.default_platform().label()
        ))
    })
}

pub fn parse_safety_catalog_file_for_platform(
    path: &str,
    raw: &str,
    platform: Platform,
) -> Result<SafetyKnowledge> {
    let catalog = parse_safety_catalog(path, raw)?;
    catalog
        .knowledge_for_platform(platform)
        .cloned()
        .ok_or_else(|| {
            RebeccaError::SafetyCatalogInvalid(format!(
                "{path} has no safety knowledge for platform {}",
                platform.label()
            ))
        })
}

pub fn default_safety_catalog() -> &'static SafetyCatalog {
    static DEFAULT: OnceLock<SafetyCatalog> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        parse_safety_catalog(
            rebecca_safety::CLEANUP_SAFETY_CATALOG_PATH,
            rebecca_safety::CLEANUP_SAFETY_CATALOG,
        )
        .expect("embedded default safety catalog should be valid")
    })
}

pub fn default_safety_knowledge() -> &'static SafetyKnowledge {
    default_safety_catalog()
        .default_knowledge()
        .expect("embedded default safety catalog should define its default platform")
}

pub fn default_safety_knowledge_for_platform(
    platform: Platform,
) -> Option<&'static SafetyKnowledge> {
    default_safety_catalog().knowledge_for_platform(platform)
}

fn compile_safety_catalog(path: &str, catalog: SafetyCatalogFile) -> Result<SafetyCatalog> {
    if catalog.catalog_version != SAFETY_CATALOG_VERSION {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} uses unsupported safety catalog version {}; expected {SAFETY_CATALOG_VERSION}",
            catalog.catalog_version
        )));
    }

    if catalog.default_platform == Platform::Unknown {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} default_platform must target a supported platform"
        )));
    }

    if catalog.platforms.is_empty() {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} must define at least one platform safety block"
        )));
    }

    let warning_kinds = catalog
        .warning_kinds
        .into_iter()
        .map(|entry| {
            let id = normalize_scalar(path, "warning_kinds.id", entry.id)?;
            let description =
                normalize_description(path, "warning_kinds.description", entry.description)?;
            Ok(WarningKind { id, description })
        })
        .collect::<Result<Vec<_>>>()?;
    validate_unique_by(path, "warning_kinds.id", &warning_kinds, WarningKind::id)?;

    let categories = catalog
        .protected_categories
        .into_iter()
        .map(|entry| {
            let description =
                normalize_description(path, "protected_categories.description", entry.description)?;
            Ok(ProtectedCategoryKnowledge {
                id: entry.id,
                description,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    validate_complete_categories(path, &categories)?;

    let maintenance_allowlist =
        compile_pattern_set(path, "maintenance_allowlist", catalog.maintenance_allowlist)?;

    let shared_protected_patterns = catalog
        .protected_patterns
        .into_iter()
        .map(|entry| compile_protected_pattern(path, entry))
        .collect::<Result<Vec<_>>>()?;

    let mut platform_knowledge = Vec::with_capacity(catalog.platforms.len());
    for platform in catalog.platforms {
        platform_knowledge.push(compile_platform_safety_knowledge(
            path,
            &warning_kinds,
            &categories,
            &maintenance_allowlist,
            &shared_protected_patterns,
            platform,
        )?);
    }

    validate_unique_platforms(path, &platform_knowledge)?;

    if !platform_knowledge
        .iter()
        .any(|knowledge| knowledge.platform == catalog.default_platform)
    {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} default platform {} has no platform safety block",
            catalog.default_platform.label()
        )));
    }

    Ok(SafetyCatalog {
        default_platform: catalog.default_platform,
        platform_knowledge,
    })
}

fn compile_platform_safety_knowledge(
    path: &str,
    warning_kinds: &[WarningKind],
    categories: &[ProtectedCategoryKnowledge],
    shared_maintenance_allowlist: &SafetyPatternSet,
    shared_protected_patterns: &[ProtectedPattern],
    platform: PlatformSafetyDefinition,
) -> Result<SafetyKnowledge> {
    if platform.platform == Platform::Unknown {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} platform safety block must target a supported platform"
        )));
    }

    validate_non_empty(
        path,
        &format!(
            "platforms.{}.critical_path_prefixes",
            platform.platform.label()
        ),
        &platform.critical_path_prefixes,
    )?;

    let critical_path_prefixes = platform
        .critical_path_prefixes
        .into_iter()
        .map(|prefix| normalize_pathish(path, "platforms.critical_path_prefixes", prefix))
        .collect::<Result<Vec<_>>>()?;
    validate_unique_strings(
        path,
        "platforms.critical_path_prefixes",
        &critical_path_prefixes,
    )?;

    let platform_maintenance_allowlist = compile_pattern_set(
        path,
        "platforms.maintenance_allowlist",
        platform.maintenance_allowlist,
    )?;
    let maintenance_allowlist = shared_maintenance_allowlist.merged(platform_maintenance_allowlist);

    let mut protected_patterns = shared_protected_patterns.to_vec();
    protected_patterns.extend(
        platform
            .protected_patterns
            .into_iter()
            .map(|entry| compile_protected_pattern(path, entry))
            .collect::<Result<Vec<_>>>()?,
    );
    validate_non_empty_patterns(path, "protected_patterns", &protected_patterns)?;

    let steam_install_allowlist = normalize_pathish_list(
        path,
        "platforms.steam_install_allowlist",
        platform.steam_install_allowlist,
    )?;
    let steam_library_allowlist = normalize_pathish_list(
        path,
        "platforms.steam_library_allowlist",
        platform.steam_library_allowlist,
    )?;

    Ok(SafetyKnowledge {
        platform: platform.platform,
        warning_kinds: warning_kinds.to_vec(),
        categories: categories.to_vec(),
        critical_path_prefixes,
        maintenance_allowlist,
        protected_patterns,
        steam_install_allowlist,
        steam_library_allowlist,
    })
}

fn compile_pattern_set(
    path: &str,
    field: &str,
    input: SafetyPatternSetFile,
) -> Result<SafetyPatternSet> {
    let segment_matches =
        normalize_scalar_list(path, &format!("{field}.segments"), input.segments)?;
    let sequence_matches =
        normalize_sequence_list(path, &format!("{field}.sequences"), input.sequences)?;

    Ok(SafetyPatternSet {
        segment_matches,
        sequence_matches,
    })
}

fn compile_protected_pattern(
    path: &str,
    input: ProtectedPatternDefinition,
) -> Result<ProtectedPattern> {
    let segment_matches =
        normalize_scalar_list(path, "protected_patterns.segments", input.segments)?;
    let leaf_names =
        normalize_scalar_list(path, "protected_patterns.leaf_names", input.leaf_names)?;
    let sequence_matches =
        normalize_sequence_list(path, "protected_patterns.sequences", input.sequences)?;

    if segment_matches.is_empty() && leaf_names.is_empty() && sequence_matches.is_empty() {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} protected pattern for {} must define at least one matcher",
            input.category.label()
        )));
    }

    Ok(ProtectedPattern {
        category: input.category,
        segment_matches,
        leaf_names,
        sequence_matches,
    })
}

fn normalize_scalar_list(path: &str, field: &str, values: Vec<String>) -> Result<Vec<String>> {
    let values = values
        .into_iter()
        .map(|value| normalize_scalar(path, field, value))
        .collect::<Result<Vec<_>>>()?;
    validate_unique_strings(path, field, &values)?;
    Ok(values)
}

fn normalize_pathish_list(path: &str, field: &str, values: Vec<String>) -> Result<Vec<String>> {
    let values = values
        .into_iter()
        .map(|value| normalize_pathish(path, field, value))
        .collect::<Result<Vec<_>>>()?;
    validate_unique_strings(path, field, &values)?;
    Ok(values)
}

fn normalize_sequence_list(
    path: &str,
    field: &str,
    values: Vec<Vec<String>>,
) -> Result<Vec<Vec<String>>> {
    let mut normalized = Vec::with_capacity(values.len());
    for sequence in values {
        if sequence.is_empty() {
            return Err(RebeccaError::SafetyCatalogInvalid(format!(
                "{path} {field} contains an empty sequence"
            )));
        }

        normalized.push(
            sequence
                .into_iter()
                .map(|value| normalize_scalar(path, field, value))
                .collect::<Result<Vec<_>>>()?,
        );
    }

    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn normalize_scalar(path: &str, field: &str, value: String) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} {field} contains an empty value"
        )));
    }
    Ok(normalized)
}

fn normalize_pathish(path: &str, field: &str, value: String) -> Result<String> {
    let normalized = value.replace('\\', "/").trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} {field} contains an empty value"
        )));
    }
    Ok(trim_trailing_separators(&normalized))
}

fn normalize_description(path: &str, field: &str, value: String) -> Result<String> {
    let description = value.trim().to_string();
    if description.is_empty() {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} {field} contains an empty value"
        )));
    }
    Ok(description)
}

fn validate_non_empty(path: &str, field: &str, values: &[String]) -> Result<()> {
    if values.is_empty() {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} {field} must not be empty"
        )));
    }
    Ok(())
}

fn validate_non_empty_patterns(path: &str, field: &str, values: &[ProtectedPattern]) -> Result<()> {
    if values.is_empty() {
        return Err(RebeccaError::SafetyCatalogInvalid(format!(
            "{path} {field} must not be empty"
        )));
    }
    Ok(())
}

fn validate_unique_strings(path: &str, field: &str, values: &[String]) -> Result<()> {
    let mut sorted = values.to_vec();
    sorted.sort();
    for window in sorted.windows(2) {
        if window[0] == window[1] {
            return Err(RebeccaError::SafetyCatalogInvalid(format!(
                "{path} {field} contains duplicate value {}",
                window[0]
            )));
        }
    }
    Ok(())
}

fn validate_unique_by<T, F>(path: &str, field: &str, values: &[T], mut key: F) -> Result<()>
where
    F: FnMut(&T) -> &str,
{
    let mut keys = values
        .iter()
        .map(|value| key(value).to_string())
        .collect::<Vec<_>>();
    keys.sort();
    for window in keys.windows(2) {
        if window[0] == window[1] {
            return Err(RebeccaError::SafetyCatalogInvalid(format!(
                "{path} {field} contains duplicate value {}",
                window[0]
            )));
        }
    }
    Ok(())
}

fn validate_unique_platforms(path: &str, values: &[SafetyKnowledge]) -> Result<()> {
    let mut labels = values
        .iter()
        .map(|knowledge| knowledge.platform.label().to_string())
        .collect::<Vec<_>>();
    labels.sort();
    for window in labels.windows(2) {
        if window[0] == window[1] {
            return Err(RebeccaError::SafetyCatalogInvalid(format!(
                "{path} platforms contains duplicate platform {}",
                window[0]
            )));
        }
    }
    Ok(())
}

fn validate_complete_categories(
    path: &str,
    categories: &[ProtectedCategoryKnowledge],
) -> Result<()> {
    const REQUIRED: &[SafetyCategory] = &[
        SafetyCategory::Credentials,
        SafetyCategory::VpnProxyState,
        SafetyCategory::AiToolDurableState,
        SafetyCategory::BrowserPrivateData,
        SafetyCategory::CloudSyncedData,
        SafetyCategory::ContainerRuntimeState,
        SafetyCategory::StartupAutomation,
        SafetyCategory::ApplicationDurableData,
    ];

    for required in REQUIRED {
        if !categories.iter().any(|category| category.id == *required) {
            return Err(RebeccaError::SafetyCatalogInvalid(format!(
                "{path} missing protected category {}",
                required.label()
            )));
        }
    }

    let mut labels = categories
        .iter()
        .map(|category| category.id.label().to_string())
        .collect::<Vec<_>>();
    labels.sort();
    for window in labels.windows(2) {
        if window[0] == window[1] {
            return Err(RebeccaError::SafetyCatalogInvalid(format!(
                "{path} protected_categories contains duplicate category {}",
                window[0]
            )));
        }
    }

    Ok(())
}

fn trim_trailing_separators(path: &str) -> String {
    let mut normalized = path.to_string();

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    normalized
}

fn has_sequence(segments: &[&str], sequence: &[String]) -> bool {
    if sequence.is_empty() || segments.len() < sequence.len() {
        return false;
    }

    segments.windows(sequence.len()).any(|window| {
        window
            .iter()
            .zip(sequence)
            .all(|(segment, expected)| segment == expected)
    })
}
