use anyhow::Result;
use rebecca::core::RuleSelection;

use crate::cli::OutputMode;

pub fn run(output_mode: OutputMode, categories: Vec<String>, rules: Vec<String>) -> Result<()> {
    let catalog = rebecca::rules::builtin_rules()?;
    let selection = RuleSelection::new(categories, rules);
    selection.validate_against_rules(&catalog)?;
    let filtered = catalog
        .iter()
        .filter(|rule| selection.matches_rule(rule))
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
