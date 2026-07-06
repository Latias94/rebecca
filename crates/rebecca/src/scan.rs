use anyhow::Result;
use rebecca::core::catalog::{CatalogItem, CatalogQuery, filter_catalog_items};
use rebecca::core::{Platform, RuleSelection};

use crate::cli::OutputMode;

pub fn run(output_mode: OutputMode, categories: Vec<String>, rules: Vec<String>) -> Result<()> {
    let catalog = rebecca::rules::builtin_rules()?;
    let item_query = CatalogQuery {
        kind: Some(rebecca::core::catalog::CatalogItemKind::CleanupRule),
        platform: Some(Platform::current()),
        categories: categories.clone(),
        rule_ids: rules.clone(),
        artifacts: Vec::new(),
        warnings: Vec::new(),
        safety_level: None,
    };
    let catalog_items = crate::catalog::cleanup_rule_catalog_items(&catalog);
    let matching_rule_ids = filter_catalog_items(catalog_items, &item_query)
        .into_iter()
        .filter_map(|item| match item {
            CatalogItem::CleanupRule(rule) => Some(rule.id),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();

    let selection = RuleSelection::new(categories, rules);
    let platform_catalog = catalog
        .iter()
        .filter(|rule| rule.platform == Platform::current())
        .cloned()
        .collect::<Vec<_>>();
    selection.validate_against_rules(&platform_catalog)?;
    let filtered = platform_catalog
        .iter()
        .filter(|rule| matching_rule_ids.contains(&rule.id) && selection.matches_rule(rule))
        .collect::<Vec<_>>();

    crate::output::print_command_success(
        "scan",
        "rule-catalog",
        output_mode,
        || &filtered,
        || {
            crate::output::print_rule_catalog(&filtered);
            Ok(())
        },
    )
}
