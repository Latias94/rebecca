mod common;

fn current_platform_prefix() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux."
    } else if cfg!(target_os = "macos") {
        "macos."
    } else {
        "windows."
    }
}

#[test]
fn scan_json_lists_current_platform_builtin_rules() {
    let output = common::command::rebecca()
        .args(["scan", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    let value: serde_json::Value = common::support::api_data(&output.stdout);
    let rules = value.as_array().expect("scan output should be an array");
    let ids = rules
        .iter()
        .map(|rule| rule["id"].as_str().expect("rule id should be a string"))
        .collect::<std::collections::BTreeSet<_>>();

    assert!(
        ids.iter()
            .all(|id| id.starts_with(current_platform_prefix())),
        "scan should only list current-platform rules: {ids:?}"
    );
    assert!(ids.contains(common::support::current_platform_user_temp_rule_id()));

    if cfg!(windows) {
        for expected in common::steam::BUILTIN_RULE_IDS
            .iter()
            .copied()
            .filter(|id| id.starts_with("windows."))
        {
            assert!(
                ids.contains(expected),
                "missing Windows scan rule {expected}"
            );
        }

        let steam_cache = rules
            .iter()
            .find(|rule| rule["id"] == "windows.steam-cache")
            .expect("steam cache rule should exist");
        assert_eq!(steam_cache["provenance"]["source"], "owned");
        assert_eq!(
            steam_cache["restore_hint"].as_str().unwrap(),
            "Steam web caches will be rebuilt on launch."
        );
    } else if cfg!(target_os = "linux") {
        assert!(ids.contains("linux.apt-cache"));
        assert!(ids.contains("linux.chrome-cache"));
        assert!(!ids.contains("windows.steam-cache"));
    }
}

#[test]
fn scan_json_filters_by_category_and_rule() {
    let rule_id = if cfg!(target_os = "linux") {
        "linux.firefox-profile-cache"
    } else {
        "windows.firefox-profile-cache"
    };

    let output = common::command::rebecca()
        .args([
            "scan",
            "--format",
            "json",
            "--category",
            "browser",
            "--rule",
            rule_id,
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let value: serde_json::Value = common::support::api_data(&output.stdout);
    let rules = value.as_array().expect("scan output should be an array");

    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0]["id"], rule_id);
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
    if cfg!(windows) {
        assert!(stdout.contains("  - windows.chrome-cache [safe] Google Chrome cache"));
        assert!(stdout.contains("  - windows.chromium-cache [safe] Chromium cache"));
        assert!(stdout.contains("  - windows.firefox-profile-cache [safe] Firefox profile cache"));
        assert!(stdout.contains("  - windows.waterfox-cache [safe] Waterfox cache"));
        assert!(stdout.contains("  - windows.zen-browser-cache [safe] Zen Browser cache"));
        assert!(stdout.contains("  - windows.postman-cache [safe] Postman cache"));
        assert!(stdout.contains("  - windows.notion-cache [safe] Notion cache"));
        assert!(stdout.contains("  - windows.figma-cache [safe] Figma cache"));
        assert!(stdout.contains("  - windows.slack-cache [safe] Slack cache"));
        assert!(stdout.contains("  - windows.zoom-logs [safe] Zoom logs"));
        assert!(stdout.contains("  - windows.teamviewer-logs [safe] TeamViewer logs"));
        assert!(stdout.contains("  - windows.vlc-cache [safe] VLC media cache"));
        assert!(stdout.contains("  - windows.thunderbird-cache [safe] Thunderbird cache"));
        assert!(stdout.contains("  - windows.adobe-reader-cache [safe] Adobe Reader cache"));
        assert!(stdout.contains("- application ("));
        assert!(stdout.contains("  - windows.wechat-cache [safe] WeChat cache"));
        assert!(stdout.contains("  - windows.wxwork-cache [safe] Enterprise WeChat cache"));
        assert!(stdout.contains("  - windows.qqmusic-cache [safe] QQ Music cache"));
        assert!(stdout.contains("  - windows.dingtalk-cache [safe] DingTalk cache"));
    } else if cfg!(target_os = "linux") {
        assert!(stdout.contains("  - linux.chrome-cache [safe] Google Chrome cache"));
        assert!(stdout.contains("  - linux.firefox-profile-cache [safe] Firefox profile cache"));
        assert!(stdout.contains("  - linux.slack-cache [safe] Slack cache"));
        assert!(stdout.contains("  - linux.apt-cache [moderate] APT package archive cache"));
        assert!(!stdout.contains("windows.chrome-cache"));
    }
    assert!(stdout.contains("- development ("));
    if cfg!(windows) {
        assert!(stdout.contains("  - windows.npm-cache [moderate] npm cache"));
        assert!(stdout.contains("  - windows.ccache-cache [moderate] ccache compiler cache"));
        assert!(stdout.contains("  - windows.sccache-cache [moderate] sccache compiler cache"));
        for expected in common::steam::HUMAN_SCAN_LINES {
            assert!(stdout.contains(expected), "{expected}");
        }
    } else if cfg!(target_os = "linux") {
        assert!(stdout.contains("  - linux.npm-cache [moderate] npm cache"));
        assert!(stdout.contains("  - linux.ccache-cache [moderate] ccache compiler cache"));
        assert!(stdout.contains("  - linux.sccache-cache [moderate] sccache compiler cache"));
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
