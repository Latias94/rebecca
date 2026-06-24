#![allow(dead_code)]

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SteamRuleCase {
    pub rule_id: &'static str,
    pub relative_path: &'static str,
    pub bytes: &'static [u8],
    pub expected_restore_hint: Option<&'static str>,
    pub allow_moderate: bool,
}

pub const BUILTIN_RULE_IDS: &[&str] = &[
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
    "windows.steam-install-depot-cache",
    "windows.steam-install-logs",
    "windows.steam-install-avatar-cache",
    "windows.steam-install-download-cache",
    "windows.steam-install-library-cache",
    "windows.steam-install-shader-cache",
    "windows.steam-library-downloading-cache",
    "windows.steam-library-shader-cache",
    "windows.steam-library-temp-cache",
    "windows.thumbnail-cache",
    "windows.user-temp",
    "windows.vscode-cache",
    "windows.wer-reports",
];

pub const HUMAN_SCAN_LINES: &[&str] = &[
    "  - windows.steam-install-depot-cache [safe] Steam install depot cache",
    "  - windows.steam-install-logs [safe] Steam install logs",
    "  - windows.steam-install-avatar-cache [safe] Steam install avatar cache [restore: Steam avatar images will be rebuilt when needed.]",
    "  - windows.steam-cache [safe] Steam cache [restore: Steam web caches will be rebuilt on launch.]",
];

pub const STEAM_INSTALL_FIXTURE_ROOT: &str = "steam-install";
pub const STEAM_LIBRARY_FIXTURE_ROOT: &str = "steam-library";

pub const STEAM_INSTALL_RULE_CASES: &[SteamRuleCase] = &[
    SteamRuleCase {
        rule_id: "windows.steam-install-cache",
        relative_path: "appcache/httpcache",
        bytes: b"ab",
        expected_restore_hint: Some("Steam client cache will be rebuilt on launch."),
        allow_moderate: false,
    },
    SteamRuleCase {
        rule_id: "windows.steam-install-depot-cache",
        relative_path: "depotcache",
        bytes: b"abcd",
        expected_restore_hint: Some("Steam depot cache will be rebuilt when Steam runs again."),
        allow_moderate: false,
    },
    SteamRuleCase {
        rule_id: "windows.steam-install-logs",
        relative_path: "logs",
        bytes: b"abc",
        expected_restore_hint: Some("Steam logs will be recreated when Steam runs again."),
        allow_moderate: false,
    },
    SteamRuleCase {
        rule_id: "windows.steam-install-avatar-cache",
        relative_path: "config/avatarcache",
        bytes: b"abc",
        expected_restore_hint: Some("Steam avatar images will be rebuilt when needed."),
        allow_moderate: false,
    },
    SteamRuleCase {
        rule_id: "windows.steam-install-download-cache",
        relative_path: "appcache/download",
        bytes: b"ab",
        expected_restore_hint: Some("Steam download staging data will be recreated if needed."),
        allow_moderate: true,
    },
    SteamRuleCase {
        rule_id: "windows.steam-install-library-cache",
        relative_path: "appcache/librarycache",
        bytes: b"ab",
        expected_restore_hint: Some(
            "Steam library artwork and metadata will be rebuilt on launch.",
        ),
        allow_moderate: false,
    },
    SteamRuleCase {
        rule_id: "windows.steam-install-shader-cache",
        relative_path: "appcache/shadercache",
        bytes: b"ab",
        expected_restore_hint: Some("Steam shader caches will be rebuilt on launch."),
        allow_moderate: false,
    },
];

pub const STEAM_LIBRARY_RULE_CASES: &[SteamRuleCase] = &[
    SteamRuleCase {
        rule_id: "windows.steam-library-shader-cache",
        relative_path: "steamapps/shadercache",
        bytes: b"ab",
        expected_restore_hint: Some("Steam shader caches will be rebuilt by Steam and games."),
        allow_moderate: false,
    },
    SteamRuleCase {
        rule_id: "windows.steam-library-downloading-cache",
        relative_path: "steamapps/downloading",
        bytes: b"ab",
        expected_restore_hint: Some("Steam download staging data will be recreated if needed."),
        allow_moderate: true,
    },
    SteamRuleCase {
        rule_id: "windows.steam-library-temp-cache",
        relative_path: "steamapps/temp",
        bytes: b"ab",
        expected_restore_hint: Some("Steam temporary staging data will be recreated if needed."),
        allow_moderate: true,
    },
];
