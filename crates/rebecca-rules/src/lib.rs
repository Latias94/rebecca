use std::collections::BTreeSet;

use rebecca_core::{
    Platform, RebeccaError, Result, RuleDefinition, RuleSource, RuleTargetSpec, SafetyLevel,
    manifest::parse_cleaner_manifest_file_with_safety_knowledge,
    planner::validate_rule_catalog,
    protection::{
        ProtectionAssessment, ProtectionPolicy, is_regenerable_browser_cache_target_shape,
    },
    safety_catalog::{SafetyKnowledge, parse_safety_catalog_file},
};

macro_rules! builtin_rule_files {
    ($($path:literal),+ $(,)?) => {
        &[
            $(
                (
                    $path,
                    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path)),
                ),
            )+
        ]
    };
}

const BUILTIN_RULE_FILES: &[(&str, &str)] = builtin_rule_files!(
    "rules/windows/user-temp.toml",
    "rules/windows/edge-cache.toml",
    "rules/windows/firefox-profile-cache.toml",
    "rules/windows/chrome-cache.toml",
    "rules/windows/chromium-cache.toml",
    "rules/windows/brave-cache.toml",
    "rules/windows/waterfox-cache.toml",
    "rules/windows/zen-browser-cache.toml",
    "rules/windows/directx-shader-cache.toml",
    "rules/windows/discord-cache.toml",
    "rules/windows/wechat-cache.toml",
    "rules/windows/wxwork-cache.toml",
    "rules/windows/qq-cache.toml",
    "rules/windows/feishu-cache.toml",
    "rules/windows/dingtalk-cache.toml",
    "rules/windows/wps-cache.toml",
    "rules/windows/baidunetdisk-cache.toml",
    "rules/windows/tencent-meeting-cache.toml",
    "rules/windows/qqmusic-cache.toml",
    "rules/windows/tencent-video-cache.toml",
    "rules/windows/postman-cache.toml",
    "rules/windows/notion-cache.toml",
    "rules/windows/figma-cache.toml",
    "rules/windows/slack-cache.toml",
    "rules/windows/zoom-logs.toml",
    "rules/windows/teamviewer-logs.toml",
    "rules/windows/vlc-cache.toml",
    "rules/windows/thunderbird-cache.toml",
    "rules/windows/adobe-reader-cache.toml",
    "rules/windows/steam-cache.toml",
    "rules/windows/steam-install-cache.toml",
    "rules/windows/steam-install-depot-cache.toml",
    "rules/windows/steam-install-logs.toml",
    "rules/windows/steam-install-avatar-cache.toml",
    "rules/windows/steam-install-stats-cache.toml",
    "rules/windows/steam-install-appinfo-cache.toml",
    "rules/windows/steam-install-localization-cache.toml",
    "rules/windows/steam-install-packageinfo-cache.toml",
    "rules/windows/steam-install-download-cache.toml",
    "rules/windows/steam-install-library-cache.toml",
    "rules/windows/steam-install-shader-cache.toml",
    "rules/windows/steam-library-shader-cache.toml",
    "rules/windows/steam-library-downloading-cache.toml",
    "rules/windows/steam-library-temp-cache.toml",
    "rules/windows/npm-cache.toml",
    "rules/windows/pnpm-cache.toml",
    "rules/windows/yarn-cache.toml",
    "rules/windows/bun-cache.toml",
    "rules/windows/corepack-cache.toml",
    "rules/windows/gradle-cache.toml",
    "rules/windows/android-cache.toml",
    "rules/windows/nuget-cache.toml",
    "rules/windows/maven-cache.toml",
    "rules/windows/pip-cache.toml",
    "rules/windows/uv-cache.toml",
    "rules/windows/poetry-cache.toml",
    "rules/windows/conda-cache.toml",
    "rules/windows/go-build-cache.toml",
    "rules/windows/go-module-cache.toml",
    "rules/windows/cargo-cache.toml",
    "rules/windows/rustup-cache.toml",
    "rules/windows/ccache-cache.toml",
    "rules/windows/sccache-cache.toml",
    "rules/windows/huggingface-cache.toml",
    "rules/windows/pytorch-cache.toml",
    "rules/windows/jetbrains-cache.toml",
    "rules/windows/thumbnail-cache.toml",
    "rules/windows/vscode-cache.toml",
    "rules/windows/wer-reports.toml",
    "rules/windows/system-temp.toml",
    "rules/windows/prefetch.toml",
    "rules/windows/update-download-cache.toml",
    "rules/windows/media-player-cache.toml",
);

const BUILTIN_SAFETY_CATALOG: (&str, &str) = (
    "safety/windows.toml",
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/safety/windows.toml")),
);

const BUILTIN_RULE_CATEGORIES: &[&str] = &["application", "browser", "development", "system"];

pub fn builtin_rules() -> Result<Vec<RuleDefinition>> {
    let mut rules = Vec::with_capacity(BUILTIN_RULE_FILES.len());
    let safety_knowledge = builtin_safety_knowledge()?;

    for (path, raw) in BUILTIN_RULE_FILES {
        let parsed_rules = parse_rule_file(path, raw, &safety_knowledge)?;
        validate_builtin_rule_file(path, &parsed_rules)?;
        rules.extend(parsed_rules);
    }

    validate_builtin_rule_catalog(&rules)?;
    validate_rule_catalog(&rules)?;
    Ok(rules)
}

pub fn builtin_safety_knowledge() -> Result<SafetyKnowledge> {
    parse_safety_catalog_file(BUILTIN_SAFETY_CATALOG.0, BUILTIN_SAFETY_CATALOG.1)
}

pub fn validate_builtin_rules() -> Result<()> {
    builtin_rules().map(|_| ())
}

fn parse_rule_file(
    path: &str,
    raw: &str,
    safety_knowledge: &SafetyKnowledge,
) -> Result<Vec<RuleDefinition>> {
    parse_cleaner_manifest_file_with_safety_knowledge(path, raw, safety_knowledge)
}

fn validate_builtin_rule_file(path: &str, rules: &[RuleDefinition]) -> Result<()> {
    let Some(stem) = builtin_rule_file_stem(path) else {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule file {path} must be a .toml file"
        )));
    };
    let expected_id = format!("windows.{stem}");
    let option_id_prefix = format!("{expected_id}.");

    if rules.is_empty() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule file {path} must compile at least one rule"
        )));
    }

    for rule in rules {
        if rule.id != expected_id && !rule.id.starts_with(&option_id_prefix) {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule file {path} produced rule {}; expected {expected_id} or {option_id_prefix}*",
                rule.id
            )));
        }
    }

    Ok(())
}

fn builtin_rule_file_stem(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\']).next()?.strip_suffix(".toml")
}

fn validate_builtin_rule_catalog(rules: &[RuleDefinition]) -> Result<()> {
    let safety_knowledge = builtin_safety_knowledge()?;
    let policy = ProtectionPolicy::new().with_safety_knowledge(&safety_knowledge);

    for rule in rules {
        if rule.platform != Platform::Windows {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must target the Windows platform",
                rule.id
            )));
        }

        if !rule.id.starts_with("windows.") {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must use a windows. rule id prefix",
                rule.id
            )));
        }

        validate_builtin_rule_metadata(rule, &safety_knowledge)?;

        if rule
            .restore_hint
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must define a restore hint",
                rule.id
            )));
        }

        if rule.provenance.source != RuleSource::Owned {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must use owned provenance source",
                rule.id
            )));
        }

        if rule.provenance.license.trim() != "project-owned" {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} must use project-owned provenance license",
                rule.id
            )));
        }

        for spec in &rule.path_templates {
            if rule.category.eq_ignore_ascii_case("browser") {
                validate_browser_cache_target_shape(rule, spec)?;
            }

            if let ProtectionAssessment::Blocked(block) = policy.assess_catalog_target_shape(spec) {
                return Err(RebeccaError::RuleCatalogInvalid(format!(
                    "built-in rule {} target {} is blocked by {}: {}",
                    rule.id,
                    spec.placeholder_path().display(),
                    block.kind.label(),
                    block.message
                )));
            }

            validate_builtin_target_shape_basis(rule, spec)?;
            validate_builtin_glob_shape(rule, spec)?;
            validate_builtin_required_shape_warnings(rule, spec)?;
        }
    }

    Ok(())
}

fn validate_builtin_rule_metadata(
    rule: &RuleDefinition,
    safety_knowledge: &SafetyKnowledge,
) -> Result<()> {
    validate_trimmed_rule_metadata(rule, "id", &rule.id)?;
    validate_trimmed_rule_metadata(rule, "category", &rule.category)?;
    validate_trimmed_rule_metadata(rule, "name", &rule.name)?;
    if let Some(restore_hint) = &rule.restore_hint {
        validate_trimmed_rule_metadata(rule, "restore hint", restore_hint)?;
    }
    validate_trimmed_rule_metadata(rule, "provenance license", &rule.provenance.license)?;
    validate_trimmed_rule_metadata(rule, "provenance notes", &rule.provenance.notes)?;
    validate_builtin_rule_provenance_notes(rule)?;

    if !is_canonical_windows_rule_id(&rule.id) {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule {} must use canonical lowercase windows.<slug> rule id syntax",
            rule.id
        )));
    }

    if !BUILTIN_RULE_CATEGORIES.contains(&rule.category.as_str()) {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule {} uses unsupported category {}; allowed categories: {}",
            rule.id,
            rule.category,
            BUILTIN_RULE_CATEGORIES.join(", ")
        )));
    }

    if matches!(
        rule.safety_level,
        SafetyLevel::Risky | SafetyLevel::Dangerous
    ) {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule {} must not use {} safety level",
            rule.id,
            rule.safety_level.label()
        )));
    }

    validate_builtin_rule_warnings(rule, safety_knowledge)
}

fn validate_trimmed_rule_metadata(rule: &RuleDefinition, field: &str, value: &str) -> Result<()> {
    if value != value.trim() {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule {} {field} must not contain leading or trailing whitespace",
            rule.id
        )));
    }

    Ok(())
}

fn validate_builtin_rule_provenance_notes(rule: &RuleDefinition) -> Result<()> {
    let lower_notes = rule.provenance.notes.to_ascii_lowercase();

    for phrase in [
        "copied from",
        "derived from",
        "imported from",
        "ported from",
    ] {
        if lower_notes.contains(phrase) {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} provenance notes must not claim copied or derived rule data",
                rule.id
            )));
        }
    }

    for source in ["bleachbit", "mole", "winapp2"] {
        if lower_notes.contains(source)
            && !(lower_notes.contains("behavior reference only")
                || lower_notes.contains("discovery index only"))
        {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} provenance notes must mark {source} as reference-only",
                rule.id
            )));
        }

        if lower_notes.contains(source) && !lower_notes.contains("no rule data copied") {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} provenance notes must state that no {source} rule data was copied",
                rule.id
            )));
        }
    }

    Ok(())
}

fn is_canonical_windows_rule_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("windows.") else {
        return false;
    };

    !rest.is_empty() && rest.split('.').all(is_rule_id_slug_segment)
}

fn is_rule_id_slug_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.starts_with('-')
        && !segment.ends_with('-')
        && !segment.contains("--")
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_builtin_rule_warnings(
    rule: &RuleDefinition,
    safety_knowledge: &SafetyKnowledge,
) -> Result<()> {
    let mut seen = BTreeSet::new();

    for warning in &rule.warnings {
        if !seen.insert(warning.as_str()) {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} contains duplicate warning kind {}",
                rule.id, warning
            )));
        }

        if !safety_knowledge
            .warning_kinds()
            .iter()
            .any(|kind| kind.id() == warning)
        {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} warning kind {} must match a canonical safety catalog warning id",
                rule.id, warning
            )));
        }
    }

    Ok(())
}

fn validate_browser_cache_target_shape(rule: &RuleDefinition, spec: &RuleTargetSpec) -> Result<()> {
    if is_regenerable_browser_cache_target_shape(spec) {
        return Ok(());
    }

    Err(RebeccaError::RuleCatalogInvalid(format!(
        "built-in browser rule {} target {} is outside the regenerable browser cache boundary",
        rule.id,
        spec.placeholder_path().display()
    )))
}

fn validate_builtin_target_shape_basis(rule: &RuleDefinition, spec: &RuleTargetSpec) -> Result<()> {
    if matches!(
        spec,
        RuleTargetSpec::SteamInstallTemplate(_) | RuleTargetSpec::SteamLibraryTemplate(_)
    ) {
        return Ok(());
    }

    if rule.category.eq_ignore_ascii_case("browser")
        && is_regenerable_browser_cache_target_shape(spec)
    {
        return Ok(());
    }

    let raw = raw_target_shape(spec);
    if has_positive_cleanup_basis(&raw) {
        return Ok(());
    }

    Err(RebeccaError::RuleCatalogInvalid(format!(
        "built-in rule {} target {} must have a positive cleanup basis such as a cache, temp, log, package-store, shader, download, or approved maintenance shape",
        rule.id,
        spec.placeholder_path().display()
    )))
}

fn validate_builtin_glob_shape(rule: &RuleDefinition, spec: &RuleTargetSpec) -> Result<()> {
    let RuleTargetSpec::GlobTemplate(template) = spec else {
        return Ok(());
    };
    let raw = normalize_rule_shape(template.raw());
    let segments = shape_segments(&raw);
    let wildcard_segments = segments
        .iter()
        .filter(|segment| contains_glob_wildcard(segment))
        .count();

    if wildcard_segments == 0 {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule {} glob target {} must contain an explicit wildcard",
            rule.id,
            spec.placeholder_path().display()
        )));
    }
    if wildcard_segments > 3 {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule {} glob target {} uses too many wildcard segments; keep discovery bounded",
            rule.id,
            spec.placeholder_path().display()
        )));
    }

    if wildcard_appears_at_profile_root(&segments) || wildcard_appears_at_drive_root(&segments) {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "built-in rule {} glob target {} starts discovery from a profile or drive root",
            rule.id,
            spec.placeholder_path().display()
        )));
    }

    Ok(())
}

fn validate_builtin_required_shape_warnings(
    rule: &RuleDefinition,
    spec: &RuleTargetSpec,
) -> Result<()> {
    for warning in required_shape_warnings(spec) {
        if !rule.warnings.iter().any(|known| known == warning) {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "built-in rule {} target {} requires warning kind {}",
                rule.id,
                spec.placeholder_path().display(),
                warning
            )));
        }
    }

    Ok(())
}

fn required_shape_warnings(spec: &RuleTargetSpec) -> Vec<&'static str> {
    let mut warnings = Vec::new();

    if matches!(
        spec,
        RuleTargetSpec::SteamInstallTemplate(_) | RuleTargetSpec::SteamLibraryTemplate(_)
    ) {
        warnings.push("source-boundary");
    }

    let raw = normalize_rule_shape(&raw_target_shape(spec));
    if raw.starts_with("%windir%/") {
        warnings.push("privileged-location");
    }

    if matches!(spec, RuleTargetSpec::GlobTemplate(_)) {
        let segments = shape_segments(&raw);
        if wildcard_requires_broad_discovery_warning(&segments) {
            warnings.push("broad-discovery");
        }
    }

    warnings
}

fn raw_target_shape(spec: &RuleTargetSpec) -> String {
    match spec {
        RuleTargetSpec::Template(template)
        | RuleTargetSpec::GlobTemplate(template)
        | RuleTargetSpec::SteamInstallTemplate(template)
        | RuleTargetSpec::SteamLibraryTemplate(template) => template.raw().to_string(),
        RuleTargetSpec::ExactPath(path) => path.to_string_lossy().into_owned(),
    }
}

fn normalize_rule_shape(raw: &str) -> String {
    raw.trim()
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

fn shape_segments(normalized: &str) -> Vec<&str> {
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn contains_glob_wildcard(segment: &str) -> bool {
    segment.contains('*') || segment.contains('?') || segment.contains('[')
}

fn wildcard_appears_at_profile_root(segments: &[&str]) -> bool {
    matches!(segments.first(), Some(root) if *root == "%userprofile%")
        && segments
            .get(1)
            .is_some_and(|segment| contains_glob_wildcard(segment))
}

fn wildcard_appears_at_drive_root(segments: &[&str]) -> bool {
    segments
        .first()
        .is_some_and(|segment| segment.ends_with(':') && segment.len() == 2)
        && segments
            .get(1)
            .is_some_and(|segment| contains_glob_wildcard(segment))
}

fn wildcard_requires_broad_discovery_warning(segments: &[&str]) -> bool {
    if star_wildcard_segment_count(segments) >= 2 {
        return true;
    }

    let mut fixed_before_first_wildcard = 0usize;
    let first_wildcard = segments
        .iter()
        .skip_while(|segment| segment.starts_with('%') && segment.ends_with('%'))
        .find(|segment| {
            if contains_glob_wildcard(segment) {
                true
            } else {
                fixed_before_first_wildcard += 1;
                false
            }
        });

    fixed_before_first_wildcard == 0
        && first_wildcard.is_some_and(|segment| *segment == "*" || *segment == "?")
}

fn star_wildcard_segment_count(segments: &[&str]) -> usize {
    segments
        .iter()
        .filter(|segment| segment.contains('*') || segment.contains('?'))
        .count()
}

fn has_positive_cleanup_basis(raw: &str) -> bool {
    let normalized = normalize_rule_shape(raw);
    let segments = shape_segments(&normalized);
    let leaf = segments.last().copied().unwrap_or_default();

    if [
        "cache",
        "caches",
        "cache2",
        "startupcache",
        "offlinecache",
        "code cache",
        "codecache",
        "gpucache",
        "dawncache",
        "graphitedawncache",
        "grshadercache",
        "shadercache",
        "d3dscache",
        "htmlcache",
        "httpcache",
        "filecache",
        "resource_cache",
        "musiccache",
        "updatecache",
        "whirlcache",
        "tmp",
        "temp",
        "%temp%",
        "logs",
        "crashdump",
        "corepack",
        "notifications",
        "image",
        "installer.txt",
        "pkgs",
        "packages",
        "repository",
        "store",
        "hub",
        "datasets",
        "assets",
        "artistalbum",
        "xet",
        "prefetch",
    ]
    .contains(&leaf)
    {
        return true;
    }

    normalized.contains("cache")
        || normalized.contains("thumbcache_")
        || normalized.contains("*.idx")
        || normalized.contains("iconcache_")
        || normalized.contains("_cacache")
        || normalized.contains("logfile.log")
        || normalized.contains("reportarchive")
        || normalized.contains("reportqueue")
        || normalized.contains("%rustup_home%/downloads")
        || normalized.contains(".rustup/downloads")
        || normalized.contains("appcache/download")
        || normalized.contains("steamapps/downloading")
        || normalized.contains("softwaredistribution/download")
        || normalized.contains("registry/cache")
        || normalized.contains("registry/index")
        || normalized.contains("registry/src")
        || normalized.contains("git/db")
        || normalized.contains("git/checkouts")
        || normalized.contains("go-build")
        || normalized.contains("pkg/mod")
        || normalized.contains("[0-9a-f]/[0-9a-f]")
        || normalized.contains("dynamicresource")
        || normalized.contains("transcoded files cache")
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs, path::Path};

    use rebecca_core::safety_catalog::default_safety_knowledge;
    use rebecca_core::{
        Platform, RuleDefinition, RuleProvenance, RuleSource, RuleTargetSpec, SafetyLevel,
    };

    use super::{builtin_rules, builtin_safety_knowledge, parse_rule_file};

    fn parse_single_rule_file(path: &str, raw: &str) -> RuleDefinition {
        let safety_knowledge =
            builtin_safety_knowledge().expect("built-in safety catalog should load");
        let rules = parse_rule_file(path, raw, &safety_knowledge).expect("test rule should parse");
        assert_eq!(rules.len(), 1);
        rules.into_iter().next().unwrap()
    }

    #[test]
    fn builtin_rule_ids_are_unique() {
        let rules = builtin_rules().expect("built-in rules should load");
        let ids = rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<HashSet<_>>();

        assert_eq!(ids.len(), rules.len());
    }

    #[test]
    fn builtin_rules_have_required_metadata() {
        super::validate_builtin_rules().expect("built-in rules should be valid");
    }

    #[test]
    fn builtin_safety_catalog_exposes_warning_and_category_knowledge() {
        let knowledge = builtin_safety_knowledge().expect("built-in safety catalog should load");

        assert!(
            knowledge
                .warning_kinds()
                .iter()
                .any(|warning| warning.id() == "active-process")
        );
        assert!(
            knowledge
                .categories()
                .iter()
                .any(|category| category.id().label() == "application-durable-data")
        );
        assert!(knowledge.is_allowed_steam_install_target("appcache/httpcache"));
        assert!(knowledge.is_allowed_steam_library_target("steamapps/downloading"));
    }

    #[test]
    fn builtin_safety_catalog_matches_core_default_catalog() {
        let builtin = builtin_safety_knowledge().expect("built-in safety catalog should load");
        let core_default = default_safety_knowledge();

        assert_eq!(builtin.platform(), core_default.platform());
        assert_eq!(
            builtin
                .warning_kinds()
                .iter()
                .map(|warning| warning.id())
                .collect::<Vec<_>>(),
            core_default
                .warning_kinds()
                .iter()
                .map(|warning| warning.id())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            builtin.critical_path_prefixes(),
            core_default.critical_path_prefixes()
        );
    }

    #[test]
    fn builtin_rules_use_owned_provenance_sources() {
        let rules = builtin_rules().expect("built-in rules should load");

        assert!(
            rules
                .iter()
                .all(|rule| rule.provenance.source == rebecca_core::RuleSource::Owned)
        );
    }

    #[test]
    fn builtin_rules_have_restore_hints_and_project_owned_provenance() {
        let rules = builtin_rules().expect("built-in rules should load");

        assert!(rules.iter().all(|rule| {
            rule.restore_hint
                .as_deref()
                .map(str::trim)
                .is_some_and(|hint| !hint.is_empty())
                && rule.provenance.license == "project-owned"
        }));
    }

    #[test]
    fn builtin_rules_are_loaded_from_toml_catalog_files() {
        let rules = builtin_rules().expect("built-in rules should load");
        let user_temp = rules
            .iter()
            .find(|rule| rule.id == "windows.user-temp")
            .expect("user temp rule should exist");

        assert_eq!(user_temp.platform, rebecca_core::Platform::Windows);
        assert_eq!(user_temp.category, "system");
        assert_eq!(user_temp.path_templates.len(), 2);
        assert_eq!(user_temp.provenance.source, RuleSource::Owned);
    }

    #[test]
    fn builtin_rule_files_match_rule_directory() {
        let rules_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("rules/windows");
        let mut discovered = fs::read_dir(rules_dir)
            .expect("built-in rule directory should be readable")
            .map(|entry| {
                entry
                    .expect("rule directory entry should be readable")
                    .path()
            })
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "toml")
            })
            .map(|path| {
                format!(
                    "rules/windows/{}",
                    path.file_name()
                        .expect("rule file should have a file name")
                        .to_string_lossy()
                )
            })
            .collect::<Vec<_>>();
        discovered.sort();

        let mut embedded = super::BUILTIN_RULE_FILES
            .iter()
            .map(|(path, _)| path.to_string())
            .collect::<Vec<_>>();
        embedded.sort();

        assert_eq!(embedded, discovered);
    }

    #[test]
    fn builtin_catalog_rejects_non_owned_provenance_sources() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::ReferenceOnly,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("owned provenance source"));
    }

    #[test]
    fn builtin_rule_file_rejects_rule_ids_that_drift_from_file_name() {
        let err = super::validate_builtin_rule_file(
            "rules/windows/user-temp.toml",
            &[rule_with_target(RuleTargetSpec::template("%TEMP%"))],
        )
        .expect_err("file name should constrain the produced rule id");

        assert!(err.to_string().contains("produced rule windows.test"));

        let option_rule = RuleDefinition {
            id: "windows.user-temp.option".to_string(),
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        };
        super::validate_builtin_rule_file("rules/windows/user-temp.toml", &[option_rule])
            .expect("option rule ids should be allowed under the file id prefix");

        let backslash_path_rule = RuleDefinition {
            id: "windows.user-temp".to_string(),
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        };
        super::validate_builtin_rule_file("rules\\windows\\user-temp.toml", &[backslash_path_rule])
            .expect("catalog path validation should accept Windows separators");
    }

    #[test]
    fn builtin_catalog_rejects_unsupported_categories() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            category: "messaging".to_string(),
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        }])
        .expect_err("unknown built-in categories should be rejected");

        assert!(err.to_string().contains("unsupported category"));
    }

    #[test]
    fn builtin_catalog_rejects_non_canonical_rule_ids() {
        for id in [
            "windows.Chrome_Cache",
            "windows.chrome--cache",
            "windows.chrome-",
        ] {
            let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
                id: id.to_string(),
                ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
            }])
            .expect_err("non-canonical rule id should be rejected");

            assert!(
                err.to_string()
                    .contains("canonical lowercase windows.<slug> rule id syntax"),
                "{err}"
            );
        }
    }

    #[test]
    fn builtin_catalog_rejects_untrimmed_metadata() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            name: " Test".to_string(),
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        }])
        .expect_err("built-in metadata should be canonicalized before shipping");

        assert!(
            err.to_string()
                .contains("must not contain leading or trailing whitespace")
        );
    }

    #[test]
    fn builtin_catalog_rejects_copied_or_derived_reference_provenance() {
        for notes in [
            "Derived from BleachBit cleaner data.",
            "Copied from upstream cleaner data.",
            "Imported from Winapp2.",
            "Ported from Mole.",
        ] {
            let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
                provenance: RuleProvenance {
                    notes: notes.to_string(),
                    ..owned_provenance()
                },
                ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
            }])
            .expect_err("built-in rules must not claim copied reference rule data");

            assert!(
                err.to_string()
                    .contains("must not claim copied or derived rule data"),
                "{err}"
            );
        }
    }

    #[test]
    fn builtin_catalog_requires_reference_only_provenance_for_restricted_sources() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            provenance: RuleProvenance {
                notes: "Cross-checked against BleachBit cleaner behavior.".to_string(),
                ..owned_provenance()
            },
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        }])
        .expect_err("restricted sources need explicit reference-only provenance");

        assert!(err.to_string().contains("reference-only"), "{err}");

        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            provenance: RuleProvenance {
                notes: "Cross-checked against BleachBit as behavior reference only.".to_string(),
                ..owned_provenance()
            },
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        }])
        .expect_err("restricted sources need no-copy provenance");

        assert!(err.to_string().contains("no bleachbit rule data"), "{err}");

        super::validate_builtin_rule_catalog(&[RuleDefinition {
            provenance: RuleProvenance {
                notes: "Cross-checked against BleachBit as behavior reference only, no rule data copied.".to_string(),
                ..owned_provenance()
            },
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        }])
        .expect("reference-only provenance should pass");
    }

    #[test]
    fn builtin_catalog_rejects_risky_and_dangerous_safety_levels() {
        for safety_level in [SafetyLevel::Risky, SafetyLevel::Dangerous] {
            let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
                safety_level,
                ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
            }])
            .expect_err("built-in rules should not require risky opt-in levels");

            assert!(err.to_string().contains("must not use"), "{err}");
        }
    }

    #[test]
    fn builtin_catalog_rejects_non_canonical_or_duplicate_warnings() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            warnings: vec!["ACTIVE-PROCESS".to_string()],
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        }])
        .expect_err("built-in warnings should use canonical ids");

        assert!(
            err.to_string()
                .contains("canonical safety catalog warning id")
        );

        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            warnings: vec!["active-process".to_string(), "active-process".to_string()],
            ..rule_with_target(RuleTargetSpec::template("%TEMP%"))
        }])
        .expect_err("duplicate warning ids should be rejected");

        assert!(err.to_string().contains("duplicate warning kind"));
    }

    #[test]
    fn builtin_catalog_rejects_targets_without_positive_cleanup_basis() {
        let err = super::validate_builtin_rule_catalog(&[rule_with_target(
            RuleTargetSpec::template("%USERPROFILE%\\Downloads"),
        )])
        .expect_err("built-in targets need a positive cleanup basis");

        assert!(err.to_string().contains("positive cleanup basis"), "{err}");
    }

    #[test]
    fn builtin_catalog_rejects_wide_profile_root_globs() {
        let err = super::validate_builtin_rule_catalog(&[rule_with_target(
            RuleTargetSpec::glob_template("%USERPROFILE%\\*\\Cache"),
        )])
        .expect_err("profile-root wildcard discovery should be rejected");

        assert!(
            err.to_string()
                .contains("starts discovery from a profile or drive root"),
            "{err}"
        );
    }

    #[test]
    fn builtin_catalog_requires_shape_implied_warnings() {
        let broad = super::validate_builtin_rule_catalog(&[rule_with_target(
            RuleTargetSpec::glob_template("%APPDATA%\\Vendor\\*\\Cache\\*\\file.tmp"),
        )])
        .expect_err("multi-wildcard glob should require broad-discovery");
        assert!(broad.to_string().contains("broad-discovery"), "{broad}");

        let mut broad_rule = rule_with_target(RuleTargetSpec::glob_template(
            "%APPDATA%\\Vendor\\*\\Cache\\*\\file.tmp",
        ));
        broad_rule.warnings = vec!["broad-discovery".to_string()];
        super::validate_builtin_rule_catalog(&[broad_rule])
            .expect("broad-discovery warning should satisfy the shape gate");

        let source = super::validate_builtin_rule_catalog(&[rule_with_target(
            RuleTargetSpec::steam_install_template("appcache\\httpcache"),
        )])
        .expect_err("Steam discovery should require source-boundary");
        assert!(source.to_string().contains("source-boundary"), "{source}");

        let mut source_rule = rule_with_target(RuleTargetSpec::steam_install_template(
            "appcache\\httpcache",
        ));
        source_rule.warnings = vec!["source-boundary".to_string()];
        super::validate_builtin_rule_catalog(&[source_rule])
            .expect("source-boundary warning should satisfy Steam discovery");

        let privileged = super::validate_builtin_rule_catalog(&[rule_with_target(
            RuleTargetSpec::template("%WINDIR%\\Temp"),
        )])
        .expect_err("Windows root maintenance targets should require privileged-location");
        assert!(
            privileged.to_string().contains("privileged-location"),
            "{privileged}"
        );

        let mut privileged_rule = rule_with_target(RuleTargetSpec::template("%WINDIR%\\Temp"));
        privileged_rule.warnings = vec!["privileged-location".to_string()];
        super::validate_builtin_rule_catalog(&[privileged_rule])
            .expect("privileged-location warning should satisfy Windows maintenance targets");
    }

    #[test]
    fn builtin_rule_fixture_matrix_catches_positive_and_near_miss_shapes() {
        super::validate_builtin_rule_catalog(&[rule_with_target(RuleTargetSpec::template(
            "%APPDATA%\\Slack\\Cache",
        ))])
        .expect("positive cache target should pass");

        let durable = super::validate_builtin_rule_catalog(&[rule_with_target(
            RuleTargetSpec::template("%APPDATA%\\Slack\\Local Storage"),
        )])
        .expect_err("durable app state near a cache should be blocked");
        assert!(durable.to_string().contains("application-durable-data"));

        let protected = super::validate_builtin_rule_catalog(&[rule_with_target(
            RuleTargetSpec::template("%USERPROFILE%\\.ssh"),
        )])
        .expect_err("protected user credential path should be blocked");
        assert!(protected.to_string().contains("credentials"));
    }

    #[test]
    fn builtin_rules_include_first_expansion_batch() {
        let rules = builtin_rules().expect("built-in rules should load");
        let ids = rules
            .iter()
            .map(|rule| rule.id.as_str())
            .collect::<HashSet<_>>();

        for expected in [
            "windows.chrome-cache",
            "windows.chromium-cache",
            "windows.android-cache",
            "windows.adobe-reader-cache",
            "windows.brave-cache",
            "windows.bun-cache",
            "windows.cargo-cache",
            "windows.ccache-cache",
            "windows.corepack-cache",
            "windows.gradle-cache",
            "windows.directx-shader-cache",
            "windows.discord-cache",
            "windows.wechat-cache",
            "windows.wxwork-cache",
            "windows.qq-cache",
            "windows.feishu-cache",
            "windows.dingtalk-cache",
            "windows.wps-cache",
            "windows.baidunetdisk-cache",
            "windows.tencent-meeting-cache",
            "windows.qqmusic-cache",
            "windows.tencent-video-cache",
            "windows.figma-cache",
            "windows.firefox-profile-cache",
            "windows.waterfox-cache",
            "windows.zen-browser-cache",
            "windows.go-build-cache",
            "windows.go-module-cache",
            "windows.huggingface-cache",
            "windows.maven-cache",
            "windows.jetbrains-cache",
            "windows.nuget-cache",
            "windows.notion-cache",
            "windows.pip-cache",
            "windows.poetry-cache",
            "windows.conda-cache",
            "windows.postman-cache",
            "windows.pytorch-cache",
            "windows.pnpm-cache",
            "windows.rustup-cache",
            "windows.sccache-cache",
            "windows.slack-cache",
            "windows.zoom-logs",
            "windows.teamviewer-logs",
            "windows.thunderbird-cache",
            "windows.vlc-cache",
            "windows.steam-cache",
            "windows.steam-install-cache",
            "windows.steam-install-download-cache",
            "windows.steam-install-library-cache",
            "windows.steam-install-shader-cache",
            "windows.steam-install-logs",
            "windows.steam-install-avatar-cache",
            "windows.steam-install-stats-cache",
            "windows.steam-install-appinfo-cache",
            "windows.steam-install-localization-cache",
            "windows.steam-install-packageinfo-cache",
            "windows.steam-library-downloading-cache",
            "windows.steam-library-shader-cache",
            "windows.steam-library-temp-cache",
            "windows.thumbnail-cache",
            "windows.uv-cache",
            "windows.vscode-cache",
            "windows.wer-reports",
            "windows.yarn-cache",
            "windows.system-temp",
            "windows.prefetch",
            "windows.update-download-cache",
            "windows.media-player-cache",
        ] {
            assert!(ids.contains(expected), "missing built-in rule: {expected}");
        }
    }

    #[test]
    fn catalog_parser_rejects_unknown_fields() {
        let safety_knowledge =
            builtin_safety_knowledge().expect("built-in safety catalog should load");
        let err = parse_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.test"
platform = "windows"
category = "system"
name = "Test"
safety_level = "safe"
unexpected = "field"

[[targets]]
kind = "template"
value = "%TEMP%"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
            &safety_knowledge,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn catalog_parser_supports_exact_path_targets() {
        let rule = parse_single_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.exact"
platform = "windows"
category = "system"
name = "Exact"
safety_level = "safe"

[[targets]]
kind = "exact-path"
value = "C:\\Users\\Example\\Cache"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        );

        assert_eq!(rule.path_templates.len(), 1);
        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::ExactPath(_)
        ));
    }

    #[test]
    fn catalog_parser_supports_glob_template_targets() {
        let rule = parse_single_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.glob"
platform = "windows"
category = "browser"
name = "Glob"
safety_level = "safe"

[[targets]]
kind = "glob-template"
value = "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        );

        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::GlobTemplate(_)
        ));
    }

    #[test]
    fn catalog_parser_supports_steam_discovery_targets() {
        let rule = parse_single_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "windows.steam-test"
platform = "windows"
category = "application"
name = "Steam test"
safety_level = "safe"

[[targets]]
kind = "steam-install-template"
value = "appcache\\httpcache"

[[targets]]
kind = "steam-library-template"
value = "steamapps\\shadercache"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
        );

        assert!(matches!(
            rule.path_templates[0],
            rebecca_core::RuleTargetSpec::SteamInstallTemplate(_)
        ));
        assert!(matches!(
            rule.path_templates[1],
            rebecca_core::RuleTargetSpec::SteamLibraryTemplate(_)
        ));
    }

    #[test]
    fn builtin_catalog_rejects_missing_restore_hints() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: None,
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("restore hint"));
    }

    #[test]
    fn builtin_catalog_rejects_non_project_owned_licenses() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "reference-only".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("project-owned provenance license"));
    }

    #[test]
    fn catalog_parser_rejects_non_windows_platforms() {
        let safety_knowledge =
            builtin_safety_knowledge().expect("built-in safety catalog should load");
        let err = parse_rule_file(
            "test.toml",
            r#"
manifest_version = 1
id = "linux.test"
platform = "linux"
category = "system"
name = "Test"
safety_level = "safe"

[[targets]]
kind = "template"
value = "/tmp"

[provenance]
source = "owned"
license = "project-owned"
notes = "test"
"#,
            &safety_knowledge,
        )
        .unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains("unknown variant") || message.contains("invalid value"),
            "{message}"
        );
    }

    #[test]
    fn builtin_catalog_rejects_non_windows_id_prefixes() {
        let err = super::validate_builtin_rule_catalog(&[RuleDefinition {
            id: "linux.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![RuleTargetSpec::template("%TEMP%")],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: RuleProvenance {
                source: RuleSource::Owned,
                license: "project-owned".to_string(),
                notes: "test".to_string(),
            },
        }])
        .unwrap_err();

        assert!(err.to_string().contains("windows. rule id prefix"));
    }

    #[test]
    fn builtin_catalog_rejects_protected_target_shapes() {
        for (target, expected) in [
            (
                RuleTargetSpec::template("%USERPROFILE%\\.ssh"),
                "credentials",
            ),
            (
                RuleTargetSpec::template(
                    "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\History",
                ),
                "browser-private-data",
            ),
            (
                RuleTargetSpec::steam_install_template("userdata"),
                "application-durable-data",
            ),
            (
                RuleTargetSpec::steam_library_template("steamapps\\common"),
                "application-durable-data",
            ),
        ] {
            let err = super::validate_builtin_rule_catalog(&[rule_with_target(target)])
                .expect_err("protected target shape should be rejected");
            assert!(
                err.to_string().contains(expected),
                "{err} should mention {expected}"
            );
        }
    }

    #[test]
    fn builtin_browser_catalog_accepts_regenerable_cache_boundary_shapes() {
        for target in [
            RuleTargetSpec::template("%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Cache"),
            RuleTargetSpec::glob_template(
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Profile *\\DawnCache",
            ),
            RuleTargetSpec::template("%LOCALAPPDATA%\\Google\\Chrome\\User Data\\ShaderCache"),
            RuleTargetSpec::template(
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\component_crx_cache",
            ),
            RuleTargetSpec::glob_template("%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2"),
            RuleTargetSpec::glob_template("%LOCALAPPDATA%\\Waterfox\\Profiles\\*\\jumpListCache"),
            RuleTargetSpec::glob_template("%LOCALAPPDATA%\\Zen\\Profiles\\*\\OfflineCache"),
        ] {
            super::validate_builtin_rule_catalog(&[browser_rule_with_target(target)])
                .expect("browser cache target shape should be accepted");
        }
    }

    #[test]
    fn builtin_browser_catalog_rejects_targets_outside_cache_boundary() {
        for target in [
            RuleTargetSpec::template("%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\History"),
            RuleTargetSpec::template(
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Preferences",
            ),
            RuleTargetSpec::template("%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Storage"),
            RuleTargetSpec::template("%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Local State"),
            RuleTargetSpec::glob_template(
                "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cookies.sqlite",
            ),
            RuleTargetSpec::glob_template("%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\storage"),
        ] {
            let err = super::validate_builtin_rule_catalog(&[browser_rule_with_target(target)])
                .expect_err("browser target outside the cache boundary should be rejected");
            assert!(
                err.to_string()
                    .contains("regenerable browser cache boundary"),
                "{err}"
            );
        }
    }

    fn browser_rule_with_target(target: RuleTargetSpec) -> RuleDefinition {
        RuleDefinition {
            category: "browser".to_string(),
            ..rule_with_target(target)
        }
    }

    fn rule_with_target(target: RuleTargetSpec) -> RuleDefinition {
        RuleDefinition {
            id: "windows.test".to_string(),
            platform: Platform::Windows,
            category: "system".to_string(),
            name: "Test".to_string(),
            safety_level: SafetyLevel::Safe,
            path_templates: vec![target],
            restore_hint: Some("Regenerated automatically.".to_string()),
            warnings: Vec::new(),
            provenance: owned_provenance(),
        }
    }

    fn owned_provenance() -> RuleProvenance {
        RuleProvenance {
            source: RuleSource::Owned,
            license: "project-owned".to_string(),
            notes: "test rule".to_string(),
        }
    }
}
