use std::path::PathBuf;

use rebecca_core::RuleTargetSpec;
use rebecca_core::config::AppPaths;
use rebecca_core::protection::{
    ProtectedCategory, ProtectionAssessment, ProtectionBlockKind, ProtectionPolicy,
};
use rebecca_core::safety::{PathDisposition, assess_existing_path, assess_path};

#[test]
fn allows_user_cache_subdirectories() {
    let disposition = assess_path(&PathBuf::from(
        "C:/Users/Alice/AppData/Local/Temp/rebecca-test",
    ));

    assert!(matches!(disposition, PathDisposition::Allowed));
}

#[test]
fn blocks_traversal_drive_roots_and_system_paths() {
    let cases = [
        "../Windows",
        "C:/",
        "C:/Windows/System32",
        "C:/Program Files/App",
        "C:/Users/Alice",
    ];

    for case in cases {
        let disposition = assess_path(&PathBuf::from(case));
        assert!(
            matches!(disposition, PathDisposition::Blocked(_)),
            "{case} should be blocked, got {disposition:?}"
        );
    }
}

#[test]
fn missing_existing_path_is_skipped() {
    let disposition = assess_existing_path(&PathBuf::from("C:/Rebecca/definitely-missing"));

    assert!(matches!(disposition, PathDisposition::Missing));
}

#[test]
fn browser_cache_paths_remain_allowed_while_private_data_is_blocked() {
    let policy = ProtectionPolicy::new();

    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/Cache"
        )),
        ProtectionAssessment::Allowed
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Local/Google/Chrome/User Data/Default/History"
        )),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::ProtectedCategory(ProtectedCategory::BrowserPrivateData)
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Roaming/Mozilla/Firefox/Profiles/abcd1234.cache2/cache2"
        )),
        ProtectionAssessment::Allowed
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Roaming/Mozilla/Firefox/Profiles/abcd1234.default/cookies.sqlite"
        )),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::ProtectedCategory(ProtectedCategory::BrowserPrivateData)
    ));
}

#[test]
fn credentials_ai_cloud_runtime_and_startup_data_are_blocked() {
    let policy = ProtectionPolicy::new();

    for (path, expected_category) in [
        (
            "C:/Users/Alice/AppData/Roaming/Microsoft/Credentials",
            ProtectedCategory::Credentials,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Clash Verge",
            ProtectedCategory::VpnProxyState,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Cursor/User",
            ProtectedCategory::AiToolDurableState,
        ),
        (
            "C:/Users/Alice/OneDrive/Documents",
            ProtectedCategory::CloudSyncedData,
        ),
        (
            "C:/Users/Alice/AppData/Local/Docker",
            ProtectedCategory::ContainerRuntimeState,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup",
            ProtectedCategory::StartupAutomation,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Steam/userdata/12345/config.vdf",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Slack/Local Storage",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Local/pypoetry/Cache/virtualenvs",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/.conda/envs/base",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/.rustup/toolchains/stable-x86_64-pc-windows-msvc",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/.android/avd/Pixel.avd",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/.android/adbkey",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/Android/Sdk/platforms/android-35",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Tencent/WeChat/bak",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Tencent/WXWork/Data/account-1/Profile",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Local/Tencent/QQ/webkit_cache",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/LarkShell/sdk_storage",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/DingTalk/userdata",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Local/Kingsoft/WPS Cloud Files/userdata/qing/cookie",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Local/Baidu/BaiduNetdisk/users/account-1/BaiduYunCacheFileV0.db",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Tencent/QQMusic/mmkv",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Tencent/QQLive/Local Storage",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Local/Tencent/QQMusic/WebkitCache",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Tencent/WeMeet/Global/Data/IndexedDB",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/LarkShell/Local Storage",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Roaming/Tencent/QQLive/Cache",
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            "C:/Users/Alice/AppData/Local/Baidu/BaiduNetdisk/users/account-1/cache",
            ProtectedCategory::ApplicationDurableData,
        ),
    ] {
        assert!(
            matches!(
                policy.assess_path(&PathBuf::from(path)),
                ProtectionAssessment::Blocked(block)
                    if block.kind == ProtectionBlockKind::ProtectedCategory(expected_category)
            ),
            "{path} should be blocked"
        );
    }
}

#[test]
fn rebecca_owned_storage_is_blocked_even_when_the_path_exists_only_logically() {
    let app_paths = AppPaths {
        config_dir: PathBuf::from("C:/Users/Alice/AppData/Roaming/Rebecca"),
        config_file: PathBuf::from("C:/Users/Alice/AppData/Roaming/Rebecca/config.toml"),
        state_dir: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/state"),
        cache_dir: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/cache"),
        history_file: PathBuf::from("C:/Users/Alice/AppData/Local/Rebecca/state/history.jsonl"),
    };
    let storage_entries = app_paths.storage_entries();
    let policy = ProtectionPolicy::new().with_protected_storage(&storage_entries);

    assert!(matches!(
        policy.assess_path(&app_paths.cache_dir.join("scan")),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::RebeccaOwnedStorage
    ));
    assert!(matches!(
        policy.assess_path(&app_paths.history_file),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::RebeccaOwnedStorage
    ));
}

#[test]
fn user_protected_paths_block_overlapping_cleanup_targets() {
    let protected_paths = vec![PathBuf::from("C:/Users/Alice/AppData/Roaming/Slack/Cache")];
    let policy = ProtectionPolicy::new().with_protected_paths(&protected_paths);

    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Roaming/Slack/Cache"
        )),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::UserProtectedPath
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Roaming/Slack/Cache/index.bin"
        )),
        ProtectionAssessment::Blocked(block)
            if block.kind == ProtectionBlockKind::UserProtectedPath
    ));
    assert!(matches!(
        policy.assess_path(&PathBuf::from(
            "C:/Users/Alice/AppData/Roaming/Slack/GPUCache"
        )),
        ProtectionAssessment::Allowed
    ));
}

#[test]
fn maintenance_allowlists_keep_known_cache_paths_open() {
    let policy = ProtectionPolicy::new();

    for path in [
        "C:/Users/Alice/.gradle/caches",
        "C:/Users/Alice/.gradle/notifications",
        "C:/Users/Alice/.m2/repository",
        "C:/Users/Alice/AppData/Local/pip/Cache",
        "C:/Users/Alice/AppData/Local/uv/cache",
        "C:/Users/Alice/AppData/Local/pypoetry/Cache/cache",
        "C:/Users/Alice/AppData/Local/pypoetry/Cache/artifacts",
        "C:/Users/Alice/.android/cache",
        "C:/Users/Alice/.android/build-cache",
        "C:/Users/Alice/AppData/Local/Google/AndroidStudio2024.2/caches",
        "C:/Users/Alice/.cache/huggingface/hub",
        "C:/Users/Alice/.cache/huggingface/datasets",
        "C:/Users/Alice/.cache/huggingface/assets",
        "C:/Users/Alice/.cache/huggingface/xet",
        "C:/Users/Alice/.cache/torch/hub",
        "C:/Users/Alice/.cache/torch/hub/checkpoints",
        "C:/Users/Alice/.conda/pkgs",
        "C:/Users/Alice/anaconda3/pkgs",
        "C:/Users/Alice/miniconda3/pkgs",
        "C:/Users/Alice/miniforge3/pkgs",
        "C:/Users/Alice/mambaforge/pkgs",
        "C:/Users/Alice/AppData/Local/go-build",
        "C:/Users/Alice/go/pkg/mod",
        "C:/Users/Alice/.ccache/0/0",
        "C:/Users/Alice/.ccache/tmp",
        "C:/Users/Alice/AppData/Local/ccache/0/0",
        "C:/Users/Alice/AppData/Local/ccache/tmp",
        "C:/Users/Alice/AppData/Roaming/ccache/0/0",
        "C:/Users/Alice/AppData/Roaming/ccache/tmp",
        "C:/Users/Alice/.rustup/downloads",
        "C:/Users/Alice/.rustup/tmp",
        "C:/Users/Alice/AppData/Local/Mozilla/sccache",
        "C:/Users/Alice/AppData/Local/JetBrains/RustRover2024.3/caches",
        "C:/Users/Alice/AppData/Roaming/Code/Cache",
        "C:/Users/Alice/AppData/Roaming/discord/GPUCache",
        "C:/Users/Alice/AppData/Roaming/Figma/Cache",
        "C:/Users/Alice/AppData/Roaming/Notion/Code Cache",
        "C:/Users/Alice/AppData/Roaming/Postman/GPUCache",
        "C:/Users/Alice/AppData/Roaming/Slack/Cache",
        "C:/Users/Alice/AppData/Roaming/Slack/Code Cache",
        "C:/Users/Alice/AppData/Roaming/Slack/GPUCache",
        "C:/Users/Alice/AppData/Roaming/Tencent/WeChat/radium/cache",
        "C:/Users/Alice/AppData/Roaming/Tencent/WeChat/WmpfCache",
        "C:/Users/Alice/AppData/Roaming/Tencent/WeChat/radium/web/profiles/multitab_abc/Cache/Cache_Data",
        "C:/Users/Alice/AppData/Roaming/Tencent/WeChat/radium/web/profiles/web_shell/Cache/Cache_Data",
        "C:/Users/Alice/AppData/Roaming/Tencent/WXWork/Data/account-1/Cache/File/crash/MEMORY.DMP",
        "C:/Users/Alice/AppData/Roaming/Tencent/WXWork/Data/account-1/Cache/Image/capture/WXWorkCapture_1.jpg",
        "C:/Users/Alice/AppData/Local/Tencent/QQ/Cache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/Cache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/Code Cache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/CodeCache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/GPUCache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/DawnCache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/GraphiteDawnCache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/GrShaderCache",
        "C:/Users/Alice/AppData/Roaming/LarkShell/ShaderCache",
        "C:/Users/Alice/AppData/Local/DingTalk_87/Cache",
        "C:/Users/Alice/AppData/Roaming/DingTalk/Cache",
        "C:/Users/Alice/AppData/Roaming/DingTalk/account-1/resource_cache",
        "C:/Users/Alice/AppData/Local/Kingsoft/Office6/cache/httpcache",
        "C:/Users/Alice/AppData/Local/Kingsoft/WPS Cloud Files/userdata/qing/filecache",
        "C:/Users/Alice/AppData/Local/Kingsoft/WPS Cloud Files/UserData/Default/WebCache/httpcache",
        "C:/Users/Alice/AppData/Local/Baidu/BaiduNetdisk/cache",
        "C:/Users/Alice/AppData/Roaming/Tencent/Meeting/Cache",
        "C:/Users/Alice/AppData/Roaming/Tencent/WeMeet/Global/Data/DynamicResourcePackage",
        "C:/Users/Alice/AppData/Roaming/Tencent/WeMeet/Global/Data/DynamicResource",
        "C:/Users/Alice/AppData/Roaming/Tencent/QQMusic/Cache",
        "C:/Users/Alice/AppData/Roaming/Tencent/QQMusic/musiccache",
        "C:/Users/Alice/AppData/Roaming/Tencent/QQMusic/updatecache",
        "C:/Users/Alice/AppData/Roaming/Tencent/QQMusic/WhirlCache",
        "C:/Users/Alice/AppData/Roaming/Tencent/QQLive/Image",
        "C:/Users/Alice/AppData/Roaming/Tencent/WeChat/radium/web/profiles/web_shell/Cache/Cache_Data",
        "C:/Users/Alice/AppData/Local/Steam/htmlcache/Default/Cache",
        "C:/Windows/Prefetch",
        "C:/Windows/SoftwareDistribution/Download",
        "C:/Windows/Temp",
        "C:/Windows/Temp/MpCmdRun.log",
        "C:/Users/Alice/AppData/Local/Microsoft/Media Player/Cache123",
        "C:/Users/Alice/AppData/Local/Microsoft/Media Player/Grafikcache/LocalMLS",
        "C:/Users/Alice/AppData/Local/Microsoft/Media Player/Transcoded Files Cache",
        "C:/Users/Alice/AppData/Local/Temp/rebecca-test",
    ] {
        assert!(
            matches!(
                policy.assess_path(&PathBuf::from(path)),
                ProtectionAssessment::Allowed
            ),
            "{path} should remain allowed"
        );
    }
}

#[test]
fn catalog_target_shapes_keep_known_maintenance_targets_open() {
    let policy = ProtectionPolicy::new();

    for target in [
        RuleTargetSpec::template("%USERPROFILE%\\.gradle\\caches"),
        RuleTargetSpec::template("%USERPROFILE%\\.gradle\\notifications"),
        RuleTargetSpec::template("%USERPROFILE%\\.m2\\repository"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\uv\\cache"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\pypoetry\\Cache\\cache"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\pypoetry\\Cache\\artifacts"),
        RuleTargetSpec::template("%ANDROID_USER_HOME%\\cache"),
        RuleTargetSpec::template("%ANDROID_USER_HOME%\\build-cache"),
        RuleTargetSpec::template("%ANDROID_SDK_HOME%\\.android\\cache"),
        RuleTargetSpec::template("%ANDROID_SDK_HOME%\\.android\\build-cache"),
        RuleTargetSpec::template("%USERPROFILE%\\.android\\cache"),
        RuleTargetSpec::template("%USERPROFILE%\\.android\\build-cache"),
        RuleTargetSpec::glob_template("%LOCALAPPDATA%\\Google\\AndroidStudio*\\caches"),
        RuleTargetSpec::template("%HF_HOME%\\hub"),
        RuleTargetSpec::template("%HF_HOME%\\datasets"),
        RuleTargetSpec::template("%HF_HOME%\\assets"),
        RuleTargetSpec::template("%HF_HOME%\\xet"),
        RuleTargetSpec::template("%HF_HUB_CACHE%"),
        RuleTargetSpec::template("%HF_DATASETS_CACHE%"),
        RuleTargetSpec::template("%HF_ASSETS_CACHE%"),
        RuleTargetSpec::template("%HF_XET_CACHE%"),
        RuleTargetSpec::template("%HUGGINGFACE_HUB_CACHE%"),
        RuleTargetSpec::template("%HUGGINGFACE_ASSETS_CACHE%"),
        RuleTargetSpec::template("%TORCH_HOME%\\hub"),
        RuleTargetSpec::template("%USERPROFILE%\\.cache\\huggingface\\hub"),
        RuleTargetSpec::template("%USERPROFILE%\\.cache\\huggingface\\datasets"),
        RuleTargetSpec::template("%USERPROFILE%\\.cache\\huggingface\\assets"),
        RuleTargetSpec::template("%USERPROFILE%\\.cache\\huggingface\\xet"),
        RuleTargetSpec::template("%USERPROFILE%\\.cache\\torch\\hub"),
        RuleTargetSpec::template("%USERPROFILE%\\.conda\\pkgs"),
        RuleTargetSpec::template("%USERPROFILE%\\anaconda3\\pkgs"),
        RuleTargetSpec::template("%USERPROFILE%\\miniconda3\\pkgs"),
        RuleTargetSpec::template("%USERPROFILE%\\miniforge3\\pkgs"),
        RuleTargetSpec::template("%USERPROFILE%\\mambaforge\\pkgs"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\go-build"),
        RuleTargetSpec::template("%USERPROFILE%\\go\\pkg\\mod"),
        RuleTargetSpec::glob_template("%CCACHE_DIR%\\[0-9a-f]\\[0-9a-f]"),
        RuleTargetSpec::template("%CCACHE_DIR%\\tmp"),
        RuleTargetSpec::glob_template("%USERPROFILE%\\.ccache\\[0-9a-f]\\[0-9a-f]"),
        RuleTargetSpec::template("%USERPROFILE%\\.ccache\\tmp"),
        RuleTargetSpec::glob_template("%LOCALAPPDATA%\\ccache\\[0-9a-f]\\[0-9a-f]"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\ccache\\tmp"),
        RuleTargetSpec::glob_template("%APPDATA%\\ccache\\[0-9a-f]\\[0-9a-f]"),
        RuleTargetSpec::template("%APPDATA%\\ccache\\tmp"),
        RuleTargetSpec::template("%RUSTUP_HOME%\\downloads"),
        RuleTargetSpec::template("%RUSTUP_HOME%\\tmp"),
        RuleTargetSpec::template("%SCCACHE_DIR%"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\Mozilla\\sccache"),
        RuleTargetSpec::template("%USERPROFILE%\\.rustup\\downloads"),
        RuleTargetSpec::template("%USERPROFILE%\\.rustup\\tmp"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Cache"),
        RuleTargetSpec::glob_template("%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cache2"),
        RuleTargetSpec::template("%APPDATA%\\Figma\\Cache"),
        RuleTargetSpec::template("%APPDATA%\\Notion\\Code Cache"),
        RuleTargetSpec::template("%APPDATA%\\Postman\\GPUCache"),
        RuleTargetSpec::template("%APPDATA%\\Slack\\Cache"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\WeChat\\radium\\cache"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\WeChat\\WmpfCache"),
        RuleTargetSpec::glob_template(
            "%APPDATA%\\Tencent\\WeChat\\radium\\web\\profiles\\multitab_*\\Cache\\Cache_Data",
        ),
        RuleTargetSpec::glob_template(
            "%APPDATA%\\Tencent\\WXWork\\Data\\*\\Cache\\File\\*\\MEMORY.DMP",
        ),
        RuleTargetSpec::template("%LOCALAPPDATA%\\Tencent\\QQ\\Cache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\Cache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\Code Cache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\CodeCache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\GPUCache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\DawnCache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\GraphiteDawnCache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\GrShaderCache"),
        RuleTargetSpec::template("%APPDATA%\\LarkShell\\ShaderCache"),
        RuleTargetSpec::glob_template("%LOCALAPPDATA%\\DingTalk*\\Cache"),
        RuleTargetSpec::glob_template("%APPDATA%\\DingTalk\\*\\resource_cache"),
        RuleTargetSpec::glob_template("%LOCALAPPDATA%\\Kingsoft\\*\\cache\\http*"),
        RuleTargetSpec::glob_template(
            "%LOCALAPPDATA%\\Kingsoft\\WPS Cloud Files\\userdata\\*\\filecache",
        ),
        RuleTargetSpec::template("%LOCALAPPDATA%\\Baidu\\BaiduNetdisk\\cache"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\Meeting\\Cache"),
        RuleTargetSpec::template(
            "%APPDATA%\\Tencent\\WeMeet\\Global\\Data\\DynamicResourcePackage",
        ),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\WeMeet\\Global\\Data\\DynamicResource"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\QQMusic\\Cache"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\QQMusic\\musiccache"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\QQMusic\\updatecache"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\QQMusic\\WhirlCache"),
        RuleTargetSpec::template("%APPDATA%\\Tencent\\QQLive\\Image"),
        RuleTargetSpec::template(
            "%APPDATA%\\Tencent\\WeChat\\radium\\web\\profiles\\web_shell\\Cache\\Cache_Data",
        ),
        RuleTargetSpec::template("%WINDIR%\\Temp"),
        RuleTargetSpec::template("%WINDIR%\\Prefetch"),
        RuleTargetSpec::template("%WINDIR%\\SoftwareDistribution\\Download"),
        RuleTargetSpec::template("%WINDIR%\\Temp"),
        RuleTargetSpec::glob_template("%LOCALAPPDATA%\\Microsoft\\Media Player\\Cache*"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\Microsoft\\Media Player\\Grafikcache\\LocalMLS"),
        RuleTargetSpec::template("%LOCALAPPDATA%\\Microsoft\\Media Player\\Transcoded Files Cache"),
        RuleTargetSpec::steam_install_template("appcache\\httpcache"),
        RuleTargetSpec::steam_install_template("logs"),
        RuleTargetSpec::steam_library_template("steamapps\\downloading"),
    ] {
        assert!(
            matches!(
                policy.assess_catalog_target_shape(&target),
                ProtectionAssessment::Allowed
            ),
            "{target:?} should be an allowed catalog target shape"
        );
    }
}

#[test]
fn catalog_target_shapes_reject_protected_categories_and_unsafe_steam_targets() {
    let policy = ProtectionPolicy::new();

    for (target, expected_category) in [
        (
            RuleTargetSpec::template("%USERPROFILE%\\.ssh"),
            ProtectedCategory::Credentials,
        ),
        (
            RuleTargetSpec::glob_template(
                "%APPDATA%\\Mozilla\\Firefox\\Profiles\\*\\cookies.sqlite",
            ),
            ProtectedCategory::BrowserPrivateData,
        ),
        (
            RuleTargetSpec::template(
                "%LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Local Storage",
            ),
            ProtectedCategory::BrowserPrivateData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Slack\\Local Storage"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Postman\\IndexedDB"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Notion\\Service Worker"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Figma\\Network"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Tencent\\WeChat\\bak"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Tencent\\WXWork\\Data"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%LOCALAPPDATA%\\Tencent\\QQ\\webkit_cache"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\LarkShell\\sdk_storage"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\DingTalk\\userdata"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template(
                "%LOCALAPPDATA%\\Kingsoft\\WPS Cloud Files\\userdata\\qing\\cookie",
            ),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%LOCALAPPDATA%\\Baidu\\BaiduNetdisk\\users"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Tencent\\QQMusic\\mmkv"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%LOCALAPPDATA%\\Tencent\\QQMusic\\WebkitCache"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Tencent\\QQLive\\IndexedDB"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Tencent\\WeMeet\\Global\\Data\\IndexedDB"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\Tencent\\QQLive\\Cache"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%USERPROFILE%\\.android\\avd"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%USERPROFILE%\\.android\\adbkey"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%USERPROFILE%\\.android\\debug.keystore"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%USERPROFILE%\\Android\\Sdk\\system-images"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%USERPROFILE%\\.conda\\envs"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%RUSTUP_HOME%\\toolchains"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%CCACHE_DIR%\\ccache.conf"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%APPDATA%\\ccache\\stats"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::template("%LOCALAPPDATA%\\ccache\\CACHEDIR.TAG"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::steam_install_template("userdata"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::steam_library_template("steamapps\\common"),
            ProtectedCategory::ApplicationDurableData,
        ),
        (
            RuleTargetSpec::steam_library_template("steamapps\\workshop"),
            ProtectedCategory::ApplicationDurableData,
        ),
    ] {
        assert!(
            matches!(
                policy.assess_catalog_target_shape(&target),
                ProtectionAssessment::Blocked(block)
                    if block.kind == ProtectionBlockKind::ProtectedCategory(expected_category)
            ),
            "{target:?} should be blocked as {expected_category:?}"
        );
    }
}
