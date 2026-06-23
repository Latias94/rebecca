use anyhow::Result;
use rebecca_core::RuleSelection;

pub fn run(json: bool, categories: Vec<String>, rules: Vec<String>) -> Result<()> {
    let catalog = rebecca_rules::builtin_rules()?;
    let selection = RuleSelection::new(categories, rules);
    selection.validate_against_rules(&catalog)?;
    let filtered = catalog
        .iter()
        .filter(|rule| selection.matches_rule(rule))
        .collect::<Vec<_>>();

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    crate::output::print_rule_catalog(&filtered);
    Ok(())
}
