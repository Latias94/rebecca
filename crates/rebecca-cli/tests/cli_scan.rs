mod common;

#[test]
fn scan_json_lists_builtin_rules() {
    let output = common::command::rebecca()
        .args(["scan", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rules = value.as_array().expect("scan output should be an array");
    let ids = rules
        .iter()
        .map(|rule| rule["id"].as_str().expect("rule id should be a string"))
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        ids,
        common::steam::BUILTIN_RULE_IDS.iter().copied().collect()
    );

    let steam_cache = rules
        .iter()
        .find(|rule| rule["id"] == "windows.steam-cache")
        .expect("steam cache rule should exist");
    assert_eq!(steam_cache["provenance"]["source"], "owned");
    assert_eq!(
        steam_cache["restore_hint"].as_str().unwrap(),
        "Steam web caches will be rebuilt on launch."
    );
}

#[test]
fn scan_json_filters_by_category_and_rule() {
    let output = common::command::rebecca()
        .args([
            "scan",
            "--json",
            "--category",
            "browser",
            "--rule",
            "windows.firefox-profile-cache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rules = value.as_array().expect("scan output should be an array");

    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["id"], "windows.firefox-profile-cache");
    assert_eq!(rules[0]["category"], "browser");
}

#[test]
fn scan_human_output_groups_rules_by_category() {
    let output = common::command::rebecca().args(["scan"]).output().unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("- browser ("));
    assert!(stdout.contains("  - windows.chrome-cache [safe] Google Chrome cache"));
    assert!(stdout.contains("  - windows.firefox-profile-cache [safe] Firefox profile cache"));
    assert!(stdout.contains("  - windows.slack-cache [safe] Slack cache"));
    assert!(stdout.contains("- development ("));
    assert!(stdout.contains("  - windows.npm-cache [moderate] npm cache"));
    assert!(stdout.contains("  - windows.sccache-cache [moderate] sccache compiler cache"));
    for expected in common::steam::HUMAN_SCAN_LINES {
        assert!(stdout.contains(expected), "{expected}");
    }
}

#[test]
fn scan_human_output_filters_by_category() {
    let output = common::command::rebecca()
        .args(["scan", "--category", "browser"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca rules: "));
    assert!(stdout.contains("- browser ("));
    assert!(!stdout.contains("- development ("));
}
