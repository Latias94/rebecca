use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{RebeccaError, Result};
use crate::manifest::parse_cleaner_manifest_file;
use crate::{Platform, RuleDefinition};

pub const EXTERNAL_RULE_STORE_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRuleIndex {
    pub version: u16,
    #[serde(default)]
    pub entries: Vec<ExternalRuleEntry>,
}

impl Default for ExternalRuleIndex {
    fn default() -> Self {
        Self {
            version: EXTERNAL_RULE_STORE_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRuleEntry {
    pub import_id: String,
    pub source_display_path: PathBuf,
    pub stored_manifest_path: PathBuf,
    pub content_hash: String,
    pub imported_at_unix_seconds: u64,
    pub enabled: bool,
    pub rule_ids: Vec<String>,
    pub platforms: Vec<Platform>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalRuleImportReport {
    pub imported: ExternalRuleEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalRuleListReport {
    pub store_dir: PathBuf,
    pub entries: Vec<ExternalRuleEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ExternalRuleStoreDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalRuleMutationReport {
    pub import_id: String,
    pub enabled: bool,
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalRuleStoreDiagnostic {
    pub import_id: Option<String>,
    pub reason_code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ExternalRuleStore {
    root_dir: PathBuf,
}

impl ExternalRuleStore {
    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn default_for_state_dir(state_dir: &Path) -> Self {
        Self::new(state_dir.join("external-rules"))
    }

    pub fn index_path(&self) -> PathBuf {
        self.root_dir.join("index.json")
    }

    pub fn manifests_dir(&self) -> PathBuf {
        self.root_dir.join("manifests")
    }

    pub fn import_manifest(&self, source: &Path) -> Result<ExternalRuleImportReport> {
        let raw = fs::read_to_string(source).map_err(|err| {
            RebeccaError::RuleCatalogInvalid(format!(
                "external rule manifest is not readable: {}: {err}",
                source.display()
            ))
        })?;
        let rules = parse_cleaner_manifest_file(&source.display().to_string(), &raw)?;
        let content_hash = content_hash(&raw);
        let import_id = content_hash.clone();
        let stored_manifest_path = self.manifests_dir().join(format!("{content_hash}.toml"));
        let mut index = self.load_index()?;

        fs::create_dir_all(self.manifests_dir())?;
        fs::write(&stored_manifest_path, raw)?;

        let entry = ExternalRuleEntry {
            import_id: import_id.clone(),
            source_display_path: source.to_path_buf(),
            stored_manifest_path,
            content_hash,
            imported_at_unix_seconds: unix_now(),
            enabled: false,
            rule_ids: sorted_rule_ids(&rules),
            platforms: sorted_platforms(&rules),
        };

        index.entries.retain(|entry| entry.import_id != import_id);
        index.entries.push(entry.clone());
        index
            .entries
            .sort_by(|left, right| left.import_id.cmp(&right.import_id));
        self.store_index(&index)?;

        Ok(ExternalRuleImportReport { imported: entry })
    }

    pub fn list(&self) -> Result<ExternalRuleListReport> {
        Ok(ExternalRuleListReport {
            store_dir: self.root_dir.clone(),
            entries: self.load_index()?.entries,
            diagnostics: Vec::new(),
        })
    }

    pub fn set_enabled(
        &self,
        import_id: &str,
        enabled: bool,
    ) -> Result<ExternalRuleMutationReport> {
        let mut index = self.load_index()?;
        let Some(entry) = index
            .entries
            .iter_mut()
            .find(|entry| entry.import_id == import_id)
        else {
            return Err(unknown_import_id(import_id));
        };
        if enabled {
            validate_stored_entry(entry)?;
        }
        entry.enabled = enabled;
        self.store_index(&index)?;
        Ok(ExternalRuleMutationReport {
            import_id: import_id.to_string(),
            enabled,
            removed: false,
        })
    }

    pub fn remove(&self, import_id: &str) -> Result<ExternalRuleMutationReport> {
        let mut index = self.load_index()?;
        let Some(position) = index
            .entries
            .iter()
            .position(|entry| entry.import_id == import_id)
        else {
            return Err(unknown_import_id(import_id));
        };
        let entry = index.entries.remove(position);
        if entry.stored_manifest_path.exists() {
            fs::remove_file(&entry.stored_manifest_path)?;
        }
        self.store_index(&index)?;
        Ok(ExternalRuleMutationReport {
            import_id: import_id.to_string(),
            enabled: false,
            removed: true,
        })
    }

    pub fn load_enabled_rules(&self) -> ExternalRuleLoadReport {
        let mut report = ExternalRuleLoadReport::default();
        let index = match self.load_index() {
            Ok(index) => index,
            Err(err) => {
                report.diagnostics.push(ExternalRuleStoreDiagnostic {
                    import_id: None,
                    reason_code: "external-rule-index-unreadable",
                    message: err.to_string(),
                });
                return report;
            }
        };

        for entry in index.entries.into_iter().filter(|entry| entry.enabled) {
            match load_stored_entry_rules(&entry) {
                Ok(rules) => report.rules.extend(rules),
                Err(err) => report.diagnostics.push(ExternalRuleStoreDiagnostic {
                    import_id: Some(entry.import_id),
                    reason_code: "external-rule-invalid",
                    message: err.to_string(),
                }),
            }
        }

        report
    }

    fn load_index(&self) -> Result<ExternalRuleIndex> {
        let path = self.index_path();
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ExternalRuleIndex::default());
            }
            Err(err) => return Err(err.into()),
        };
        let index = serde_json::from_str::<ExternalRuleIndex>(&raw)?;
        if index.version != EXTERNAL_RULE_STORE_VERSION {
            return Err(RebeccaError::RuleCatalogInvalid(format!(
                "{} uses unsupported external rule store version {}; expected {EXTERNAL_RULE_STORE_VERSION}",
                path.display(),
                index.version
            )));
        }
        Ok(index)
    }

    fn store_index(&self, index: &ExternalRuleIndex) -> Result<()> {
        fs::create_dir_all(&self.root_dir)?;
        fs::write(self.index_path(), serde_json::to_vec_pretty(index)?)?;
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExternalRuleLoadReport {
    pub rules: Vec<RuleDefinition>,
    pub diagnostics: Vec<ExternalRuleStoreDiagnostic>,
}

fn validate_stored_entry(entry: &ExternalRuleEntry) -> Result<()> {
    load_stored_entry_rules(entry).map(|_| ())
}

fn load_stored_entry_rules(entry: &ExternalRuleEntry) -> Result<Vec<RuleDefinition>> {
    let raw = fs::read_to_string(&entry.stored_manifest_path)?;
    let current_hash = content_hash(&raw);
    if current_hash != entry.content_hash {
        return Err(RebeccaError::RuleCatalogInvalid(format!(
            "stored external rule {} content hash changed",
            entry.import_id
        )));
    }
    parse_cleaner_manifest_file(&entry.stored_manifest_path.display().to_string(), &raw)
}

fn unknown_import_id(import_id: &str) -> RebeccaError {
    RebeccaError::RuleCatalogInvalid(format!("unknown external rule import id: {import_id}"))
}

fn sorted_rule_ids(rules: &[RuleDefinition]) -> Vec<String> {
    let mut ids = rules.iter().map(|rule| rule.id.clone()).collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn sorted_platforms(rules: &[RuleDefinition]) -> Vec<Platform> {
    let mut platforms = rules.iter().map(|rule| rule.platform).collect::<Vec<_>>();
    platforms.sort_by_key(|platform| platform.label());
    platforms.dedup();
    platforms
}

fn content_hash(raw: &str) -> String {
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> String {
        r#"
manifest_version = 1
id = "example-cache"
category = "development"
name = "Example cache"
safety_level = "safe"
restore_hint = "Example rebuilds this cache automatically."

[provenance]
source = "reference-only"
license = "example-user-rule"
notes = "Local user-authored validation fixture; no external rule data copied."

[[platforms]]
platform = "macos"

[[platforms.targets]]
kind = "template"
value = "MACOS_CACHE_HOME/Example"
search_kind = "file"
"#
        .to_string()
    }

    #[test]
    fn import_stores_manifest_disabled_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("example.toml");
        fs::write(&source, valid_manifest()).unwrap();
        let store = ExternalRuleStore::new(temp.path().join("store"));

        let report = store.import_manifest(&source).unwrap();

        assert!(!report.imported.enabled);
        assert_eq!(report.imported.rule_ids, ["macos.example-cache"]);
        assert!(report.imported.stored_manifest_path.exists());
        assert!(store.list().unwrap().entries[0].content_hash.len() >= 16);
    }

    #[test]
    fn enabled_rules_are_revalidated_before_loading() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("example.toml");
        fs::write(&source, valid_manifest()).unwrap();
        let store = ExternalRuleStore::new(temp.path().join("store"));
        let import_id = store.import_manifest(&source).unwrap().imported.import_id;
        store.set_enabled(&import_id, true).unwrap();

        let report = store.load_enabled_rules();

        assert!(report.diagnostics.is_empty());
        assert_eq!(report.rules[0].id, "macos.example-cache");
    }

    #[test]
    fn corrupted_stored_manifest_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("example.toml");
        fs::write(&source, valid_manifest()).unwrap();
        let store = ExternalRuleStore::new(temp.path().join("store"));
        let imported = store.import_manifest(&source).unwrap().imported;
        store.set_enabled(&imported.import_id, true).unwrap();
        fs::write(&imported.stored_manifest_path, "manifest_version = 1\n").unwrap();

        let report = store.load_enabled_rules();

        assert!(report.rules.is_empty());
        assert_eq!(report.diagnostics[0].reason_code, "external-rule-invalid");
    }
}
