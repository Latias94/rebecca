use std::path::Path;

use super::ProtectedCategory;

const ELECTRON_CACHE_APPS: &[&str] = &[
    "code",
    "discord",
    "discordptb",
    "discordcanary",
    "figma",
    "notion",
    "postman",
    "slack",
];
const ELECTRON_CACHE_DIRS: &[&str] = &["cache", "code cache", "gpucache", "cacheddata"];

pub(super) struct NormalizedPath {
    pub(super) raw: String,
    pub(super) lower: String,
}

impl NormalizedPath {
    pub(super) fn new(path: &Path) -> Self {
        let raw =
            trim_trailing_separators(path.as_os_str().to_string_lossy().replace('\\', "/").trim());
        let lower = raw.to_ascii_lowercase();

        Self { raw, lower }
    }

    pub(super) fn segments(&self) -> Vec<&str> {
        self.lower
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect()
    }
}

pub(super) fn normalize_shape_path(path: &Path) -> String {
    normalize_raw_shape(&path.as_os_str().to_string_lossy())
}

pub(super) fn normalize_raw_shape(raw: &str) -> String {
    let replaced = raw.replace('\\', "/");
    trim_trailing_separators(replaced.trim()).to_ascii_lowercase()
}

pub(super) fn looks_absolute_shape(normalized: &str) -> bool {
    normalized.starts_with('/') || normalized.as_bytes().get(1) == Some(&b':')
}

pub(super) fn contains_relative_control_segment(normalized: &str) -> bool {
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .any(|segment| matches!(segment, "." | ".."))
}

pub(super) fn is_allowed_steam_install_catalog_shape(normalized: &str) -> bool {
    matches!(
        normalized,
        "appcache/httpcache"
            | "appcache/download"
            | "appcache/librarycache"
            | "appcache/shadercache"
            | "appcache/stats"
            | "appcache/appinfo.vdf"
            | "appcache/localization.vdf"
            | "appcache/packageinfo.vdf"
            | "config/avatarcache"
            | "depotcache"
            | "logs"
    )
}

pub(super) fn is_allowed_steam_library_catalog_shape(normalized: &str) -> bool {
    matches!(
        normalized,
        "steamapps/shadercache" | "steamapps/downloading" | "steamapps/temp"
    )
}

pub(super) fn contains_traversal(normalized: &str) -> bool {
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .any(|segment| segment == "..")
}

pub(super) fn is_root(normalized: &str) -> bool {
    if normalized == "/" || normalized == "//" {
        return true;
    }

    if normalized.len() == 2 {
        let bytes = normalized.as_bytes();
        return bytes[1] == b':' && bytes[0].is_ascii_alphabetic();
    }

    if normalized.len() == 3 {
        let bytes = normalized.as_bytes();
        return bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/';
    }

    if let Some(rest) = normalized.strip_prefix("//") {
        return rest
            .split('/')
            .filter(|segment| !segment.is_empty())
            .count()
            == 2;
    }

    false
}

pub(super) fn is_windows_critical_path(lower: &str) -> bool {
    const PROTECTED_PREFIXES: &[&str] = &[
        "c:/windows",
        "c:/program files",
        "c:/program files (x86)",
        "c:/programdata",
        "c:/$recycle.bin",
        "c:/recovery",
        "c:/system volume information",
    ];

    PROTECTED_PREFIXES
        .iter()
        .any(|prefix| same_or_descendant(lower, prefix))
}

pub(super) fn is_user_profile_root(lower: &str) -> bool {
    let mut parts = lower.split('/').filter(|segment| !segment.is_empty());
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(drive), Some("users"), Some(_name), None) if drive.ends_with(':')
    )
}

pub(super) fn is_allowlisted_maintenance_path(path: &NormalizedPath) -> bool {
    let segments = path.segments();

    is_chromium_cache_path(&segments)
        || is_firefox_cache_path(&segments)
        || is_electron_cache_path(&segments)
        || is_jetbrains_cache_path(&segments)
        || is_cargo_cache_path(&segments)
        || is_ccache_cache_path(&segments)
        || is_conda_cache_path(&segments)
        || is_rustup_cache_path(&segments)
        || is_sccache_cache_path(&segments)
        || is_go_cache_path(&segments)
        || is_android_cache_path(&segments)
        || is_domestic_desktop_app_cache_path(&segments)
        || is_python_package_manager_cache_path(&segments)
        || is_node_package_manager_cache_path(&segments)
        || is_dotnet_package_manager_cache_path(&segments)
        || is_gradle_cache_path(&segments)
        || is_maven_cache_path(&segments)
        || is_windows_maintenance_cache_path(&segments)
        || is_known_temp_or_report_path(&segments)
}

pub(super) fn protected_category(path: &NormalizedPath) -> Option<ProtectedCategory> {
    let segments = path.segments();

    if is_credential_path(&segments) {
        return Some(ProtectedCategory::Credentials);
    }

    if is_vpn_or_proxy_path(&segments) {
        return Some(ProtectedCategory::VpnProxyState);
    }

    if is_ai_tool_durable_state_path(&segments) {
        return Some(ProtectedCategory::AiToolDurableState);
    }

    if is_browser_private_data_path(&segments) {
        return Some(ProtectedCategory::BrowserPrivateData);
    }

    if is_cloud_synced_path(&segments) {
        return Some(ProtectedCategory::CloudSyncedData);
    }

    if is_container_runtime_path(&segments) {
        return Some(ProtectedCategory::ContainerRuntimeState);
    }

    if is_startup_automation_path(&segments) {
        return Some(ProtectedCategory::StartupAutomation);
    }

    if is_application_durable_data_path(&segments) {
        return Some(ProtectedCategory::ApplicationDurableData);
    }

    None
}

pub(super) fn is_app_leftover_cache_path(path: &NormalizedPath) -> bool {
    let segments = path.segments();
    segments.windows(4).any(|window| {
        matches!(window[0], "appdata")
            && matches!(window[1], "local" | "roaming" | "locallow")
            && is_specific_leftover_app_segment(window[2])
            && is_rebuildable_leftover_leaf(window[3])
    })
}

fn trim_trailing_separators(path: &str) -> String {
    let mut normalized = path.to_string();

    while normalized.ends_with('/') && normalized.len() > 3 {
        normalized.pop();
    }

    normalized
}

fn is_chromium_cache_path(segments: &[&str]) -> bool {
    find_segment(segments, "user data")
        .is_some_and(|index| chromium_cache_tail_is_allowed(segments, index + 1))
        || find_segment(segments, "htmlcache")
            .is_some_and(|index| chromium_cache_tail_is_allowed(segments, index + 1))
}

fn is_firefox_cache_path(segments: &[&str]) -> bool {
    find_sequence(segments, &["mozilla", "firefox", "profiles"]).is_some_and(|index| {
        segments
            .get(index + 4)
            .is_some_and(|segment| matches!(*segment, "cache2" | "startupcache"))
    })
}

fn is_electron_cache_path(segments: &[&str]) -> bool {
    let Some(appdata_index) = find_segment(segments, "appdata") else {
        return false;
    };

    let app = segments.get(appdata_index + 2).copied().unwrap_or_default();
    let cache = segments.get(appdata_index + 3).copied().unwrap_or_default();

    ELECTRON_CACHE_APPS.contains(&app) && ELECTRON_CACHE_DIRS.contains(&cache)
}

fn is_jetbrains_cache_path(segments: &[&str]) -> bool {
    find_segment(segments, "jetbrains").is_some_and(|index| {
        segments
            .get(index + 2)
            .is_some_and(|segment| *segment == "caches")
    })
}

fn is_android_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".android", "cache"])
        || has_sequence(segments, &[".android", "build-cache"])
        || has_sequence(segments, &["%android_user_home%", "cache"])
        || has_sequence(segments, &["%android_user_home%", "build-cache"])
        || find_segment(segments, "google").is_some_and(|index| {
            segments
                .get(index + 1)
                .is_some_and(|segment| segment.starts_with("androidstudio"))
                && segments
                    .get(index + 2)
                    .is_some_and(|segment| *segment == "caches")
        })
}

fn is_domestic_desktop_app_cache_path(segments: &[&str]) -> bool {
    is_wechat_cache_path(segments)
        || is_wxwork_cache_path(segments)
        || is_qq_cache_path(segments)
        || is_feishu_cache_path(segments)
        || is_dingtalk_cache_path(segments)
        || is_wps_cache_path(segments)
        || is_baidu_netdisk_cache_path(segments)
        || is_tencent_meeting_cache_path(segments)
        || is_qqmusic_cache_path(segments)
        || is_tencent_video_cache_path(segments)
}

fn is_wechat_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["tencent", "wechat", "radium", "cache"])
        || has_sequence(segments, &["tencent", "wechat", "wmpfcache"])
        || find_sequence(
            segments,
            &["tencent", "wechat", "radium", "web", "profiles"],
        )
        .is_some_and(|index| {
            segments
                .get(index + 5)
                .is_some_and(|segment| segment.starts_with("multitab_") || *segment == "web_shell")
                && segments
                    .get(index + 6)
                    .is_some_and(|segment| *segment == "cache")
                && segments
                    .get(index + 7)
                    .is_some_and(|segment| *segment == "cache_data")
        })
}

fn is_wxwork_cache_path(segments: &[&str]) -> bool {
    find_sequence(segments, &["tencent", "wxwork", "data"]).is_some_and(|index| {
        segments
            .get(index + 4)
            .is_some_and(|segment| *segment == "cache")
            && segments
                .get(index + 5)
                .is_some_and(|segment| matches!(*segment, "file" | "image"))
    })
}

fn is_qq_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["tencent", "qq", "cache"])
}

fn is_feishu_cache_path(segments: &[&str]) -> bool {
    find_segment(segments, "larkshell").is_some_and(|index| {
        segments.get(index + 1).is_some_and(|segment| {
            matches!(
                *segment,
                "cache"
                    | "code cache"
                    | "codecache"
                    | "gpucache"
                    | "dawncache"
                    | "graphitedawncache"
                    | "grshadercache"
                    | "shadercache"
            )
        })
    })
}

fn is_dingtalk_cache_path(segments: &[&str]) -> bool {
    segments.iter().enumerate().any(|(index, segment)| {
        segment.starts_with("dingtalk")
            && (segments.get(index + 1).is_some_and(|next| *next == "cache")
                || segments
                    .get(index + 2)
                    .is_some_and(|next| *next == "resource_cache"))
    })
}

fn is_wps_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["kingsoft", "wps cloud files", "userdata"])
        && segments.contains(&"filecache")
        || find_segment(segments, "kingsoft").is_some_and(|index| {
            segments
                .get(index + 2)
                .is_some_and(|segment| *segment == "cache")
                && segments
                    .get(index + 3)
                    .is_some_and(|segment| segment.starts_with("http"))
        })
        || has_sequence(
            segments,
            &[
                "kingsoft",
                "wps cloud files",
                "userdata",
                "default",
                "webcache",
            ],
        ) && segments
            .last()
            .is_some_and(|segment| segment.starts_with("http"))
}

fn is_baidu_netdisk_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["baidu", "baidunetdisk", "cache"])
}

fn is_tencent_meeting_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["tencent", "meeting", "cache"])
        || has_sequence(
            segments,
            &[
                "tencent",
                "wemeet",
                "global",
                "data",
                "dynamicresourcepackage",
            ],
        )
        || has_sequence(
            segments,
            &["tencent", "wemeet", "global", "data", "dynamicresource"],
        )
}

fn is_qqmusic_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["tencent", "qqmusic", "cache"])
        || has_sequence(segments, &["tencent", "qqmusic", "musiccache"])
        || has_sequence(segments, &["tencent", "qqmusic", "updatecache"])
        || has_sequence(segments, &["tencent", "qqmusic", "whirlcache"])
}

fn is_tencent_video_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["tencent", "qqlive", "image"])
}

fn is_cargo_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["registry", "cache"])
        || has_sequence(segments, &["registry", "index"])
        || has_sequence(segments, &["registry", "src"])
        || has_sequence(segments, &["git", "db"])
        || has_sequence(segments, &["git", "checkouts"])
}

fn is_ccache_cache_path(segments: &[&str]) -> bool {
    ccache_root_index(segments).is_some_and(|index| {
        segments.get(index + 1).is_some_and(|segment| {
            *segment == "tmp"
                || segments.get(index + 2).is_some_and(|segment2| {
                    (is_hex_bucket_segment(segment) && is_hex_bucket_segment(segment2))
                        || (is_hex_bucket_shape_segment(segment)
                            && is_hex_bucket_shape_segment(segment2))
                })
        })
    })
}

fn is_go_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["appdata", "local", "go-build"])
        || has_sequence(segments, &["go", "pkg", "mod"])
}

fn is_rustup_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".rustup", "downloads"]) || has_sequence(segments, &[".rustup", "tmp"])
}

fn is_sccache_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["appdata", "local", "mozilla", "sccache"])
}

fn is_conda_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".conda", "pkgs"])
        || has_sequence(segments, &["anaconda3", "pkgs"])
        || has_sequence(segments, &["miniconda3", "pkgs"])
        || has_sequence(segments, &["miniforge3", "pkgs"])
        || has_sequence(segments, &["mambaforge", "pkgs"])
}

fn is_python_package_manager_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["pip", "cache"])
        || has_sequence(segments, &["appdata", "local", "uv", "cache"])
        || has_sequence(
            segments,
            &["appdata", "local", "pypoetry", "cache", "cache"],
        )
        || has_sequence(
            segments,
            &["appdata", "local", "pypoetry", "cache", "artifacts"],
        )
}

fn is_npm_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["npm-cache", "_cacache"])
}

fn is_node_package_manager_cache_path(segments: &[&str]) -> bool {
    is_npm_cache_path(segments)
        || has_sequence(segments, &["appdata", "local", "pnpm", "store"])
        || has_sequence(segments, &["appdata", "local", "yarn", "cache"])
        || has_sequence(segments, &[".bun", "install", "cache"])
        || has_sequence(segments, &["appdata", "local", "node", "corepack"])
}

fn is_dotnet_package_manager_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".nuget", "packages"])
        || has_sequence(segments, &["appdata", "local", "nuget", "v3-cache"])
        || has_sequence(segments, &["appdata", "local", "nuget", "plugins-cache"])
        || has_sequence(segments, &["appdata", "local", "nuget", "cache"])
}

fn is_gradle_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".gradle", "caches"])
        || has_sequence(segments, &[".gradle", "notifications"])
}

fn is_maven_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".m2", "repository"])
}

fn is_known_temp_or_report_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["appdata", "local", "temp"])
        || has_sequence(segments, &["microsoft", "windows", "wer", "reportarchive"])
        || has_sequence(segments, &["microsoft", "windows", "wer", "reportqueue"])
}

fn is_windows_maintenance_cache_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["windows", "temp"])
        || has_sequence(segments, &["windows", "prefetch"])
        || has_sequence(segments, &["windows", "softwaredistribution", "download"])
}

fn is_specific_leftover_app_segment(segment: &str) -> bool {
    segment.chars().filter(|ch| ch.is_alphanumeric()).count() >= 3
        && !matches!(
            segment,
            "microsoft" | "windows" | "programs" | "packages" | "temp" | "cache" | "data"
        )
}

fn is_rebuildable_leftover_leaf(segment: &str) -> bool {
    matches!(segment, "cache" | "code cache" | "gpucache" | "cacheddata")
}

fn is_browser_cache_segment(segment: &str) -> bool {
    matches!(segment, "cache" | "code cache" | "gpucache")
}

fn chromium_cache_tail_is_allowed(segments: &[&str], start: usize) -> bool {
    matches!(
        (segments.get(start), segments.get(start + 1)),
        (Some(&"default"), Some(cache)) if is_browser_cache_segment(cache)
    ) || matches!(
        (segments.get(start), segments.get(start + 1)),
        (Some(profile), Some(cache))
            if profile.starts_with("profile ") && is_browser_cache_segment(cache)
    )
}

fn is_credential_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["microsoft", "credentials"])
        || has_sequence(segments, &["microsoft", "protect"])
        || has_sequence(segments, &["microsoft", "crypto"])
        || has_sequence(segments, &["microsoft", "vault"])
        || has_any_segment(segments, &[".ssh", ".gnupg", "1password", "bitwarden"])
        || segments
            .last()
            .is_some_and(|leaf| matches!(*leaf, "credentials.toml" | "key4.db" | "logins.json"))
}

fn is_vpn_or_proxy_path(segments: &[&str]) -> bool {
    has_any_segment(
        segments,
        &[
            "clash",
            "clash verge",
            "tailscale",
            "wireguard",
            "v2ray",
            "shadowsocks",
            "nekoray",
            "sing-box",
        ],
    )
}

fn is_ai_tool_durable_state_path(segments: &[&str]) -> bool {
    has_any_segment(segments, &[".codex", ".claude", ".cursor", ".ollama"])
        || has_any_segment(segments, &["claude", "cursor", "ollama", "chatgpt"])
        || has_sequence(segments, &["code", "user"])
}

fn is_browser_private_data_path(segments: &[&str]) -> bool {
    if find_segment(segments, "user data").is_some_and(|index| {
        segments.get(index + 2).is_some_and(|segment| {
            matches!(
                *segment,
                "history"
                    | "cookies"
                    | "login data"
                    | "web data"
                    | "local storage"
                    | "indexeddb"
                    | "service worker"
                    | "network"
            )
        })
    }) {
        return true;
    }

    find_sequence(segments, &["mozilla", "firefox", "profiles"]).is_some_and(|index| {
        segments.get(index + 4).is_some_and(|segment| {
            matches!(
                *segment,
                "cookies.sqlite"
                    | "places.sqlite"
                    | "logins.json"
                    | "key4.db"
                    | "formhistory.sqlite"
                    | "storage"
            )
        })
    })
}

fn is_cloud_synced_path(segments: &[&str]) -> bool {
    has_any_segment(
        segments,
        &[
            "onedrive",
            "icloud drive",
            "icloud photos",
            "dropbox",
            "google drive",
            "box",
            "mega",
        ],
    )
}

fn is_container_runtime_path(segments: &[&str]) -> bool {
    has_any_segment(
        segments,
        &[
            ".docker",
            ".podman",
            ".kube",
            ".wslconfig",
            "docker",
            "docker desktop",
            "podman",
            "rancher desktop",
            "orbstack",
        ],
    )
}

fn is_startup_automation_path(segments: &[&str]) -> bool {
    has_sequence(
        segments,
        &["microsoft", "windows", "start menu", "programs", "startup"],
    )
}

fn is_application_durable_data_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["steam", "userdata"])
        || has_sequence(segments, &["steamapps", "common"])
        || has_sequence(segments, &["steamapps", "workshop"])
        || has_sequence(segments, &["steamapps", "compatdata"])
        || has_sequence(segments, &["pypoetry", "cache", "virtualenvs"])
        || is_domestic_desktop_app_durable_state_path(segments)
        || is_ccache_durable_state_path(segments)
        || is_conda_durable_state_path(segments)
        || is_rustup_durable_state_path(segments)
        || is_android_durable_state_path(segments)
        || has_any_segment(
            segments,
            &["local storage", "indexeddb", "service worker", "network"],
        )
}

fn is_domestic_desktop_app_durable_state_path(segments: &[&str]) -> bool {
    has_sequence(segments, &["tencent", "wechat"])
        || has_sequence(segments, &["tencent", "wxwork"])
        || has_sequence(segments, &["tencent", "qq"])
        || has_sequence(segments, &["tencent", "meeting"])
        || has_sequence(segments, &["tencent", "wemeet"])
        || has_sequence(segments, &["tencent", "qqmusic"])
        || has_sequence(segments, &["tencent", "qqlive"])
        || has_sequence(segments, &["baidu", "baidunetdisk"])
        || has_sequence(segments, &["kingsoft", "wps cloud files"])
        || has_sequence(segments, &["kingsoft", "office"])
        || has_sequence(segments, &["larkshell"])
        || segments
            .iter()
            .any(|segment| segment.starts_with("dingtalk"))
}

fn is_ccache_durable_state_path(segments: &[&str]) -> bool {
    ccache_root_index(segments).is_some_and(|index| {
        segments
            .iter()
            .skip(index + 1)
            .any(|segment| matches!(*segment, "ccache.conf" | "stats" | "cachedir.tag"))
    })
}

fn is_conda_durable_state_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".conda", "envs"])
        || has_sequence(segments, &["anaconda3", "envs"])
        || has_sequence(segments, &["miniconda3", "envs"])
        || has_sequence(segments, &["miniforge3", "envs"])
        || has_sequence(segments, &["mambaforge", "envs"])
}

fn is_rustup_durable_state_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".rustup", "toolchains"])
        || has_sequence(segments, &[".rustup", "settings.toml"])
        || has_sequence(segments, &[".rustup", "overrides"])
        || has_sequence(segments, &[".rustup", "update-hashes"])
        || has_sequence(segments, &["%rustup_home%", "toolchains"])
        || has_sequence(segments, &["%rustup_home%", "settings.toml"])
        || has_sequence(segments, &["%rustup_home%", "overrides"])
        || has_sequence(segments, &["%rustup_home%", "update-hashes"])
}

fn is_android_durable_state_path(segments: &[&str]) -> bool {
    has_sequence(segments, &[".android", "avd"])
        || has_sequence(segments, &[".android", "adbkey"])
        || has_sequence(segments, &[".android", "adbkey.pub"])
        || has_sequence(segments, &[".android", "debug.keystore"])
        || has_sequence(segments, &[".android", "repositories.cfg"])
        || has_sequence(segments, &["android", "sdk", "platforms"])
        || has_sequence(segments, &["android", "sdk", "platform-tools"])
        || has_sequence(segments, &["android", "sdk", "build-tools"])
        || has_sequence(segments, &["android", "sdk", "system-images"])
        || has_sequence(segments, &["android", "sdk", "ndk"])
        || has_sequence(segments, &["android", "sdk", "licenses"])
}

fn same_or_descendant(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn has_sequence(segments: &[&str], sequence: &[&str]) -> bool {
    find_sequence(segments, sequence).is_some()
}

fn find_sequence(segments: &[&str], sequence: &[&str]) -> Option<usize> {
    if sequence.is_empty() || segments.len() < sequence.len() {
        return None;
    }

    segments
        .windows(sequence.len())
        .position(|window| window == sequence)
}

fn find_segment(segments: &[&str], needle: &str) -> Option<usize> {
    segments.iter().position(|segment| *segment == needle)
}

fn has_any_segment(segments: &[&str], needles: &[&str]) -> bool {
    segments
        .iter()
        .any(|segment| needles.iter().any(|needle| segment == needle))
}

fn ccache_root_index(segments: &[&str]) -> Option<usize> {
    find_segment(segments, "%ccache_dir%")
        .or_else(|| find_segment(segments, "ccache"))
        .or_else(|| find_segment(segments, ".ccache"))
}

fn is_hex_bucket_segment(segment: &str) -> bool {
    segment.len() == 1 && segment.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_hex_bucket_shape_segment(segment: &str) -> bool {
    matches!(segment, "[0-9a-f]" | "[0-9A-F]")
}
