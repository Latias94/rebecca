use anyhow::Result;
use rebecca::core::catalog::{
    ActionKindCatalogItem, CatalogItem, CatalogItemKind, CatalogQuery, CleanupRuleCatalogItem,
    ProjectArtifactCatalogItem, SafetyCategoryCatalogItem, WarningCatalogItem,
    filter_catalog_items,
};
use rebecca::core::project_artifacts::all_project_artifact_policies;
use rebecca::core::{RuleDefinition, SafetyLevel};

use crate::cli::OutputMode;

#[derive(Debug)]
pub struct CatalogOptions {
    pub output_mode: OutputMode,
    pub kind: Option<CatalogItemKind>,
    pub categories: Vec<String>,
    pub rules: Vec<String>,
    pub artifacts: Vec<String>,
    pub warnings: Vec<String>,
    pub safety_level: Option<SafetyLevel>,
}

pub fn run(options: CatalogOptions) -> Result<()> {
    let rules = rebecca::rules::builtin_rules()?;
    let safety_knowledge = rebecca::rules::builtin_safety_knowledge()?;
    let catalog = build_catalog_items(&rules, &safety_knowledge);
    let query = catalog_query(&options);
    let filtered = filter_catalog_items(catalog, &query);

    validate_catalog_selection(&filtered, &options)?;

    crate::output::print_command_success_with_api_version(
        crate::output::API_VERSION_V2,
        "catalog",
        "catalog",
        options.output_mode,
        || &filtered,
        || {
            print_catalog(&filtered);
            Ok(())
        },
    )
}

pub(crate) fn cleanup_rule_catalog_items(rules: &[RuleDefinition]) -> Vec<CatalogItem> {
    rules
        .iter()
        .map(|rule| CatalogItem::CleanupRule(CleanupRuleCatalogItem::from(rule)))
        .collect()
}

pub(crate) fn project_artifact_catalog_items() -> Vec<CatalogItem> {
    all_project_artifact_policies()
        .map(ProjectArtifactCatalogItem::from)
        .map(CatalogItem::ProjectArtifact)
        .collect()
}

fn build_catalog_items(
    rules: &[RuleDefinition],
    safety_knowledge: &rebecca::core::safety_catalog::SafetyKnowledge,
) -> Vec<CatalogItem> {
    let mut items = cleanup_rule_catalog_items(rules);
    items.extend(project_artifact_catalog_items());
    items.extend(
        safety_knowledge
            .warning_kinds()
            .iter()
            .map(WarningCatalogItem::from)
            .map(CatalogItem::Warning),
    );
    items.extend(
        safety_knowledge
            .categories()
            .iter()
            .map(SafetyCategoryCatalogItem::from)
            .map(CatalogItem::SafetyCategory),
    );
    items.push(CatalogItem::ActionKind(ActionKindCatalogItem::delete()));
    items
}

fn catalog_query(options: &CatalogOptions) -> CatalogQuery {
    CatalogQuery {
        kind: options.kind,
        categories: options.categories.clone(),
        rule_ids: options.rules.clone(),
        artifacts: options.artifacts.clone(),
        warnings: options.warnings.clone(),
        safety_level: options.safety_level,
    }
}

fn validate_catalog_selection(items: &[CatalogItem], options: &CatalogOptions) -> Result<()> {
    let filters = options.categories.len()
        + options.rules.len()
        + options.artifacts.len()
        + options.warnings.len()
        + usize::from(options.safety_level.is_some())
        + usize::from(options.kind.is_some());

    if filters > 0 && items.is_empty() {
        return Err(rebecca::core::RebeccaError::InvalidCatalogSelector(
            "catalog selection did not match any items".to_string(),
        )
        .into());
    }

    Ok(())
}

fn print_catalog(items: &[CatalogItem]) {
    println!("Rebecca catalog: {}", items.len());

    if items.is_empty() {
        println!("No catalog entries match the current selection.");
        return;
    }

    let mut current_kind = None::<CatalogItemKind>;
    for item in items {
        let kind = item.kind();
        if current_kind.as_ref() != Some(&kind) {
            current_kind = Some(kind);
            println!("- {}:", kind.label());
        }

        match item {
            CatalogItem::CleanupRule(rule) => {
                println!(
                    "  - {} [{}] {} ({}, {} target{}){}",
                    rule.id,
                    rule.safety_level.label(),
                    rule.name,
                    rule.category,
                    rule.targets,
                    if rule.targets == 1 { "" } else { "s" },
                    warning_suffix(&rule.warnings),
                );
            }
            CatalogItem::ProjectArtifact(artifact) => {
                println!(
                    "  - {} ({}, {}; ranking: {})",
                    artifact.artifact, artifact.rule_suffix, artifact.rule_id, artifact.ranking
                );
            }
            CatalogItem::Warning(warning) => {
                println!("  - {}: {}", warning.id, warning.description);
            }
            CatalogItem::SafetyCategory(category) => {
                println!("  - {}: {}", category.id, category.description);
            }
            CatalogItem::ActionKind(action) => {
                println!("  - {}: {}", action.id, action.description);
            }
        }
    }
}

fn warning_suffix(warnings: &[String]) -> String {
    if warnings.is_empty() {
        String::new()
    } else {
        format!(" [warnings: {}]", warnings.join(", "))
    }
}
