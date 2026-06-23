use std::process::Command;

#[test]
fn scan_json_lists_builtin_rules() {
    let output = rebecca().args(["scan", "--json"]).output().unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rules = value.as_array().expect("scan output should be an array");
    let ids = rules
        .iter()
        .map(|rule| rule["id"].as_str().expect("rule id should be a string"))
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(
        ids,
        [
            "windows.chrome-cache",
            "windows.cargo-cache",
            "windows.directx-shader-cache",
            "windows.discord-cache",
            "windows.edge-cache",
            "windows.firefox-profile-cache",
            "windows.jetbrains-cache",
            "windows.npm-cache",
            "windows.pip-cache",
            "windows.steam-cache",
            "windows.steam-install-cache",
            "windows.steam-install-download-cache",
            "windows.steam-install-library-cache",
            "windows.steam-library-downloading-cache",
            "windows.steam-library-shader-cache",
            "windows.steam-library-temp-cache",
            "windows.thumbnail-cache",
            "windows.user-temp",
            "windows.vscode-cache",
            "windows.wer-reports",
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn scan_json_filters_by_category_and_rule() {
    let output = rebecca()
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

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rules = value.as_array().expect("scan output should be an array");

    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["id"], "windows.firefox-profile-cache");
    assert_eq!(rules[0]["category"], "browser");
}

#[test]
fn scan_human_output_groups_rules_by_category() {
    let output = rebecca().args(["scan"]).output().unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("- browser ("));
    assert!(stdout.contains("  - windows.chrome-cache [Safe] Google Chrome cache"));
    assert!(stdout.contains("  - windows.firefox-profile-cache [Safe] Firefox profile cache"));
    assert!(stdout.contains("- development ("));
    assert!(stdout.contains("  - windows.npm-cache [Moderate] npm cache"));
}

#[test]
fn scan_human_output_filters_by_category() {
    let output = rebecca()
        .args(["scan", "--category", "browser"])
        .output()
        .unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Rebecca rules: "));
    assert!(stdout.contains("- browser ("));
    assert!(!stdout.contains("- development ("));
}

fn rebecca() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rebecca"))
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
