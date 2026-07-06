use std::path::Path;

use super::ProtectedCategory;
use crate::model::RuleTargetSpec;
use crate::safety_catalog::{SafetyCategory, SafetyKnowledge};

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

pub(super) fn is_regenerable_browser_cache_target_shape(spec: &RuleTargetSpec) -> bool {
    let raw = spec.placeholder_path().to_string_lossy().to_string();
    let normalized = normalize_raw_shape(&raw);
    let segments = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    is_chromium_browser_cache_target_shape(&segments)
        || is_gecko_browser_cache_target_shape(&segments)
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

pub(super) fn is_critical_path(lower: &str, knowledge: &SafetyKnowledge) -> bool {
    knowledge
        .critical_path_prefixes()
        .iter()
        .any(|prefix| same_or_descendant(lower, prefix))
}

pub(super) fn is_user_profile_root(lower: &str) -> bool {
    let mut parts = lower.split('/').filter(|segment| !segment.is_empty());
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(drive), Some("users"), Some(_name), None) if drive.ends_with(':') => true,
        (Some("home"), Some(_name), None, None) => true,
        (Some("users"), Some(_name), None, None) => true,
        (Some("root"), None, None, None) => true,
        _ => false,
    }
}

pub(super) fn is_allowlisted_maintenance_path(
    path: &NormalizedPath,
    knowledge: &SafetyKnowledge,
) -> bool {
    let segments = path.segments();

    knowledge.maintenance_allowlist().matches(&segments)
        || is_chromium_cache_path(&segments)
        || is_gecko_profile_cache_path(&segments)
        || is_electron_cache_path(&segments)
        || is_jetbrains_cache_path(&segments)
        || is_ccache_cache_path(&segments)
        || is_android_cache_path(&segments)
        || is_domestic_desktop_app_cache_path(&segments)
}

pub(super) fn protected_category(
    path: &NormalizedPath,
    knowledge: &SafetyKnowledge,
) -> Option<ProtectedCategory> {
    let segments = path.segments();

    if catalog_matches_category(knowledge, SafetyCategory::Credentials, &segments) {
        return Some(ProtectedCategory::Credentials);
    }

    if catalog_matches_category(knowledge, SafetyCategory::VpnProxyState, &segments) {
        return Some(ProtectedCategory::VpnProxyState);
    }

    if catalog_matches_category(knowledge, SafetyCategory::AiToolDurableState, &segments) {
        return Some(ProtectedCategory::AiToolDurableState);
    }

    if is_browser_private_data_path(&segments) {
        return Some(ProtectedCategory::BrowserPrivateData);
    }

    if catalog_matches_category(knowledge, SafetyCategory::CloudSyncedData, &segments) {
        return Some(ProtectedCategory::CloudSyncedData);
    }

    if catalog_matches_category(knowledge, SafetyCategory::ContainerRuntimeState, &segments) {
        return Some(ProtectedCategory::ContainerRuntimeState);
    }

    if catalog_matches_category(knowledge, SafetyCategory::StartupAutomation, &segments) {
        return Some(ProtectedCategory::StartupAutomation);
    }

    if catalog_matches_category(knowledge, SafetyCategory::ApplicationDurableData, &segments)
        || is_application_durable_data_path(&segments)
    {
        return Some(ProtectedCategory::ApplicationDurableData);
    }

    None
}

fn catalog_matches_category(
    knowledge: &SafetyKnowledge,
    category: SafetyCategory,
    segments: &[&str],
) -> bool {
    knowledge
        .protected_patterns()
        .iter()
        .any(|pattern| pattern.category() == category && pattern.matches(segments))
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
        .is_some_and(|index| chromium_user_data_cache_tail_is_allowed(segments, index + 1))
        || find_segment(segments, "htmlcache")
            .is_some_and(|index| chromium_profile_cache_tail_is_allowed(segments, index + 1))
}

fn is_gecko_profile_cache_path(segments: &[&str]) -> bool {
    let Some(index) = find_segment(segments, "profiles") else {
        return false;
    };

    if !gecko_profile_root_is_allowed(segments, index) {
        return false;
    }

    segments
        .get(index + 2)
        .is_some_and(|segment| is_gecko_profile_cache_segment(segment))
}

fn gecko_profile_root_is_allowed(segments: &[&str], profiles_index: usize) -> bool {
    match (
        profiles_index
            .checked_sub(2)
            .and_then(|index| segments.get(index)),
        profiles_index
            .checked_sub(1)
            .and_then(|index| segments.get(index)),
    ) {
        (Some(&"mozilla"), Some(&"firefox")) => true,
        (_, Some(app)) => {
            matches!(*app, "firefox" | "waterfox" | "zen" | "thunderbird")
        }
        _ => false,
    }
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

fn is_chromium_profile_cache_segment(segment: &str) -> bool {
    matches!(
        segment,
        "cache" | "code cache" | "gpucache" | "dawncache" | "media cache"
    )
}

fn is_chromium_root_cache_segment(segment: &str) -> bool {
    matches!(
        segment,
        "component_crx_cache"
            | "graphitedawncache"
            | "grshadercache"
            | "shadercache"
            | "extensions_crx_cache"
    )
}

fn is_chromium_browser_cache_target_shape(segments: &[&str]) -> bool {
    let Some(user_data_index) = find_segment(segments, "user data") else {
        return false;
    };

    match &segments[user_data_index + 1..] {
        [cache] => is_chromium_root_cache_segment(cache),
        [profile, cache] => {
            is_chromium_profile_segment(profile) && is_chromium_profile_cache_segment(cache)
        }
        _ => false,
    }
}

fn chromium_user_data_cache_tail_is_allowed(segments: &[&str], start: usize) -> bool {
    segments
        .get(start)
        .is_some_and(|segment| is_chromium_root_cache_segment(segment))
        || chromium_profile_cache_tail_is_allowed(segments, start)
}

fn chromium_profile_cache_tail_is_allowed(segments: &[&str], start: usize) -> bool {
    matches!(
        (segments.get(start), segments.get(start + 1)),
        (Some(&"default"), Some(cache)) if is_chromium_profile_cache_segment(cache)
    ) || matches!(
        (segments.get(start), segments.get(start + 1)),
        (Some(profile), Some(cache))
            if profile.starts_with("profile ") && is_chromium_profile_cache_segment(cache)
    )
}

fn is_chromium_profile_segment(segment: &str) -> bool {
    segment == "default" || segment == "profile *" || segment.starts_with("profile ")
}

fn is_gecko_browser_cache_target_shape(segments: &[&str]) -> bool {
    let Some(profiles_index) = find_segment(segments, "profiles") else {
        return false;
    };

    gecko_profile_root_is_allowed(segments, profiles_index)
        && matches!(
            &segments[profiles_index + 1..],
            [profile, cache] if !profile.is_empty() && is_gecko_profile_cache_segment(cache)
        )
}

fn is_gecko_profile_cache_segment(segment: &str) -> bool {
    matches!(
        segment,
        "cache2" | "startupcache" | "jumplistcache" | "offlinecache"
    )
}

fn is_browser_private_data_path(segments: &[&str]) -> bool {
    if find_segment(segments, "user data")
        .is_some_and(|index| is_chromium_private_data_tail(segments, index + 1))
    {
        return true;
    }

    find_segment(segments, "profiles").is_some_and(|index| {
        if !gecko_profile_root_is_allowed(segments, index) {
            return false;
        }

        segments.get(index + 2).is_some_and(|segment| {
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

fn is_chromium_private_data_tail(segments: &[&str], start: usize) -> bool {
    if segments.get(start).is_some_and(|segment| {
        *segment == "local state"
            || segment.starts_with("safe browsing")
            || segment.starts_with("variations")
    }) {
        return true;
    }

    let profile = segments.get(start).copied().unwrap_or_default();
    if profile != "default" && !profile.starts_with("profile ") {
        return false;
    }

    segments
        .get(start + 1)
        .is_some_and(|segment| is_chromium_profile_private_data_segment(segment))
}

fn is_chromium_profile_private_data_segment(segment: &str) -> bool {
    matches!(
        segment,
        "bookmarks"
            | "cookies"
            | "favicons"
            | "history"
            | "indexeddb"
            | "login data"
            | "local storage"
            | "network"
            | "preferences"
            | "service worker"
            | "session storage"
            | "sessions"
            | "sync data"
            | "top sites"
            | "visited links"
            | "web data"
    )
}

fn is_application_durable_data_path(segments: &[&str]) -> bool {
    is_domestic_desktop_app_durable_state_path(segments) || is_ccache_durable_state_path(segments)
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
