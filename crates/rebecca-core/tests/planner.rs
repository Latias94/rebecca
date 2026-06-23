use std::fs;
use std::path::Path;

use rebecca_core::environment::MapEnvironment;
use rebecca_core::planner::{
    PlanProgressEvent, build_cleanup_plan_with_environment,
    build_cleanup_plan_with_environment_and_progress,
};
use rebecca_core::{DeleteMode, PlanRequest, Platform, TargetStatus};

#[test]
fn category_filter_includes_only_matching_rules() {
    let fixture = PlannerFixture::new();
    fixture.write("temp/a.tmp", b"abc");
    fixture.write("local/Temp/b.tmp", b"de");
    fixture.write(
        "local/Microsoft/Edge/User Data/Default/Cache/cache.bin",
        b"edge",
    );
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_categories = vec!["system".to_string()];

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert!(
        plan.targets
            .iter()
            .any(|target| target.rule_id == "windows.user-temp")
    );
    assert!(plan.targets.iter().all(|target| {
        matches!(
            target.rule_id.as_str(),
            "windows.user-temp"
                | "windows.directx-shader-cache"
                | "windows.thumbnail-cache"
                | "windows.wer-reports"
        )
    }));
    assert!(
        !plan
            .targets
            .iter()
            .any(|target| target.rule_id == "windows.edge-cache")
    );
}

#[test]
fn overlapping_templates_are_deduplicated_before_sizing() {
    let fixture = PlannerFixture::with_local_temp_as_temp();
    fixture.write("local/Temp/a.tmp", b"abc");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["windows.user-temp".to_string()];

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert_eq!(plan.summary.total_targets, 2);
    assert_eq!(plan.summary.allowed_targets, 1);
    assert_eq!(plan.summary.skipped_targets, 1);
    assert_eq!(plan.summary.estimated_bytes, 3);
    assert!(
        plan.targets
            .iter()
            .any(|target| target.reason.as_deref() == Some("duplicate target path already covered"))
    );
}

#[test]
fn planner_reports_target_scan_progress() {
    let fixture = PlannerFixture::new();
    fixture.write("temp/a.tmp", b"abc");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["windows.user-temp".to_string()];

    let mut events = Vec::new();
    let plan =
        build_cleanup_plan_with_environment_and_progress(&request, &rules, &fixture.env, |event| {
            match event {
                PlanProgressEvent::TargetScanning { rule_id, path } => {
                    events.push(format!("scanning:{rule_id}:{}", path.display()));
                }
                PlanProgressEvent::TargetFinished {
                    rule_id,
                    status,
                    estimated_bytes,
                    ..
                } => {
                    events.push(format!("finished:{rule_id}:{status:?}:{estimated_bytes}"));
                }
            }
        })
        .unwrap();

    assert_eq!(plan.summary.allowed_targets, 1);
    assert!(
        events
            .iter()
            .any(|event| event.starts_with("scanning:windows.user-temp:"))
    );
    assert!(
        events
            .iter()
            .any(|event| event == "finished:windows.user-temp:Allowed:3")
    );
    assert!(
        events
            .iter()
            .any(|event| event == "finished:windows.user-temp:Skipped:0")
    );
}

#[test]
fn glob_rules_expand_profile_and_file_patterns() {
    let fixture = PlannerFixture::new();
    fixture.write("roaming/Mozilla/Firefox/Profiles/alice/cache2/a.bin", b"a");
    fixture.write(
        "roaming/Mozilla/Firefox/Profiles/alice/startupCache/startup.bin",
        b"bc",
    );
    fixture.write("roaming/Mozilla/Firefox/Profiles/bob/cache2/b.bin", b"def");
    fixture.write(
        "local/Microsoft/Windows/Explorer/thumbcache_96.db",
        b"thumb",
    );
    fixture.write("local/Microsoft/Windows/Explorer/iconcache_32.db", b"icon");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec![
        "windows.firefox-profile-cache".to_string(),
        "windows.thumbnail-cache".to_string(),
    ];

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert_eq!(plan.summary.allowed_targets, 5);
    assert_eq!(plan.summary.estimated_bytes, 15);
    assert!(
        plan.targets
            .iter()
            .any(|target| target.path.ends_with("alice/cache2"))
    );
    assert!(
        plan.targets
            .iter()
            .any(|target| target.path.ends_with("bob/cache2"))
    );
    assert!(
        plan.targets
            .iter()
            .any(|target| target.path.ends_with("thumbcache_96.db"))
    );
}

#[test]
fn jetbrains_rules_expand_product_directories() {
    let fixture = PlannerFixture::new();
    fixture.write("local/JetBrains/IntelliJIdea2026.1/caches/index.bin", b"ab");
    fixture.write("local/JetBrains/Rider2025.3/caches/index.bin", b"cde");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["windows.jetbrains-cache".to_string()];
    request.allow_moderate = true;

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert_eq!(plan.summary.allowed_targets, 2);
    assert_eq!(plan.summary.skipped_targets, 7);
    assert_eq!(plan.summary.estimated_bytes, 5);
    assert!(plan.targets.iter().any(|target| {
        target
            .path
            .ends_with(Path::new("IntelliJIdea2026.1").join("caches"))
    }));
    assert!(plan.targets.iter().any(|target| {
        target
            .path
            .ends_with(Path::new("Rider2025.3").join("caches"))
    }));
}

#[test]
fn cargo_rule_targets_default_cargo_home_cache_directories() {
    let fixture = PlannerFixture::new();
    fixture.write("user/.cargo/registry/cache/index.crate", b"ab");
    fixture.write("user/.cargo/registry/index/index/.cache", b"xyz");
    fixture.write("user/.cargo/registry/src/package/lib.rs", b"cde");
    fixture.write("user/.cargo/git/db/repo/HEAD", b"fghi");
    fixture.write("user/.cargo/git/checkouts/repo/main.rs", b"jklmn");
    fixture.write("user/.cargo/bin/tool.exe", b"do not target binaries");
    fixture.write("user/.cargo/credentials.toml", b"do not target credentials");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["windows.cargo-cache".to_string()];
    request.allow_moderate = true;

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();
    let allowed_paths = plan
        .targets
        .iter()
        .filter(|target| target.status == TargetStatus::Allowed)
        .map(|target| target.path.clone())
        .collect::<Vec<_>>();

    assert_eq!(plan.summary.allowed_targets, 5);
    assert_eq!(plan.summary.skipped_targets, 5);
    assert_eq!(plan.summary.estimated_bytes, 17);
    assert!(allowed_paths.iter().all(|path| {
        path.ends_with(Path::new("registry").join("cache"))
            || path.ends_with(Path::new("registry").join("index"))
            || path.ends_with(Path::new("registry").join("src"))
            || path.ends_with(Path::new("git").join("db"))
            || path.ends_with(Path::new("git").join("checkouts"))
    }));
}

#[test]
fn cargo_rule_targets_custom_cargo_home_cache_directories() {
    let fixture = PlannerFixture::with_cargo_home();
    fixture.write("cargo-home/registry/cache/index.crate", b"ab");
    fixture.write("cargo-home/registry/index/index/.cache", b"xyz");
    fixture.write("cargo-home/registry/src/package/lib.rs", b"cde");
    fixture.write("cargo-home/git/db/repo/HEAD", b"fghi");
    fixture.write("cargo-home/git/checkouts/repo/main.rs", b"jklmn");
    fixture.write("cargo-home/bin/tool.exe", b"do not target binaries");
    fixture.write("cargo-home/credentials.toml", b"do not target credentials");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["windows.cargo-cache".to_string()];
    request.allow_moderate = true;

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert_eq!(plan.summary.allowed_targets, 5);
    assert_eq!(plan.summary.skipped_targets, 5);
    assert_eq!(plan.summary.estimated_bytes, 17);
    assert!(
        plan.targets
            .iter()
            .filter(|target| target.status == TargetStatus::Allowed)
            .all(|target| target.path.starts_with(fixture.root.join("cargo-home")))
    );
}

#[test]
fn discord_rule_targets_only_browser_cache_directories() {
    let fixture = PlannerFixture::new();
    fixture.write("roaming/discord/Cache/cache.bin", b"ab");
    fixture.write("roaming/discord/Code Cache/code.bin", b"cde");
    fixture.write("roaming/discord/GPUCache/gpu.bin", b"fghi");
    fixture.write("roaming/discordptb/Cache/cache.bin", b"jklmn");
    fixture.write("roaming/discordptb/Code Cache/code.bin", b"opqrst");
    fixture.write("roaming/discordptb/GPUCache/gpu.bin", b"uvwxyz1");
    fixture.write("roaming/discord/Local Storage/leveldb/LOG", b"keep");
    fixture.write("roaming/discord/IndexedDB/indexeddb.leveldb/LOG", b"keep");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["windows.discord-cache".to_string()];

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert_eq!(plan.summary.allowed_targets, 6);
    assert_eq!(plan.summary.skipped_targets, 3);
    assert_eq!(plan.summary.estimated_bytes, 27);
    assert!(plan.targets.iter().all(|target| {
        target.path.ends_with(Path::new("Cache"))
            || target.path.ends_with(Path::new("Code Cache"))
            || target.path.ends_with(Path::new("GPUCache"))
    }));
    assert!(plan.targets.iter().all(|target| {
        !target.path.to_string_lossy().contains("Local Storage")
            && !target.path.to_string_lossy().contains("IndexedDB")
    }));
}

#[test]
fn chromium_rules_expand_profile_cache_patterns() {
    let fixture = PlannerFixture::new();
    fixture.write(
        "local/Google/Chrome/User Data/Default/Cache/default.bin",
        b"a",
    );
    fixture.write(
        "local/Google/Chrome/User Data/Default/Code Cache/default-code.bin",
        b"bb",
    );
    fixture.write(
        "local/Google/Chrome/User Data/Default/GPUCache/default-gpu.bin",
        b"ccc",
    );
    fixture.write(
        "local/Google/Chrome/User Data/Profile 1/Cache/profile-cache.bin",
        b"dddd",
    );
    fixture.write(
        "local/Google/Chrome/User Data/Profile 1/Code Cache/profile-code.bin",
        b"eeeee",
    );
    fixture.write(
        "local/Google/Chrome/User Data/Profile 1/GPUCache/profile-gpu.bin",
        b"ffffff",
    );
    fixture.write(
        "local/Microsoft/Edge/User Data/Default/Cache/default.bin",
        b"1234567",
    );
    fixture.write(
        "local/Microsoft/Edge/User Data/Default/Code Cache/default-code.bin",
        b"12345678",
    );
    fixture.write(
        "local/Microsoft/Edge/User Data/Default/GPUCache/default-gpu.bin",
        b"123456789",
    );
    fixture.write(
        "local/Microsoft/Edge/User Data/Profile 2/Cache/profile-cache.bin",
        b"1234567890",
    );
    fixture.write(
        "local/Microsoft/Edge/User Data/Profile 2/Code Cache/profile-code.bin",
        b"12345678901",
    );
    fixture.write(
        "local/Microsoft/Edge/User Data/Profile 2/GPUCache/profile-gpu.bin",
        b"123456789012",
    );
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec![
        "windows.chrome-cache".to_string(),
        "windows.edge-cache".to_string(),
    ];

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert_eq!(plan.summary.allowed_targets, 12);
    assert_eq!(plan.summary.estimated_bytes, 78);
    assert!(plan.targets.iter().any(|target| {
        target
            .path
            .ends_with(Path::new("Profile 1").join("Code Cache"))
    }));
    assert!(plan.targets.iter().any(|target| {
        target
            .path
            .ends_with(Path::new("Profile 2").join("GPUCache"))
    }));
}

#[test]
fn unknown_rule_id_is_an_error() {
    let fixture = PlannerFixture::new();
    let rules = rebecca_rules::builtin_rules().unwrap();
    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["missing.rule".to_string()];

    let err = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap_err();

    assert!(err.to_string().contains("invalid rule id"));
}

#[test]
fn moderate_rule_is_skipped_without_opt_in() {
    let fixture = PlannerFixture::new();
    fixture.write("roaming/npm-cache/_cacache/index.bin", b"npm");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    request.selected_rule_ids = vec!["windows.npm-cache".to_string()];

    let plan = build_cleanup_plan_with_environment(&request, &rules, &fixture.env).unwrap();

    assert_eq!(plan.summary.skipped_targets, 1);
    assert_eq!(plan.targets[0].status, TargetStatus::Skipped);
}

#[test]
fn dry_run_and_recycle_bin_share_target_set() {
    let fixture = PlannerFixture::new();
    fixture.write("temp/a.tmp", b"abc");
    let rules = rebecca_rules::builtin_rules().unwrap();

    let mut dry_request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun);
    dry_request.selected_rule_ids = vec!["windows.user-temp".to_string()];

    let mut run_request = PlanRequest::for_platform(Platform::Windows, DeleteMode::RecycleBin);
    run_request.selected_rule_ids = vec!["windows.user-temp".to_string()];

    let dry_plan = build_cleanup_plan_with_environment(&dry_request, &rules, &fixture.env).unwrap();
    let run_plan = build_cleanup_plan_with_environment(&run_request, &rules, &fixture.env).unwrap();

    let dry_paths = dry_plan
        .targets
        .iter()
        .map(|target| target.path.clone())
        .collect::<Vec<_>>();
    let run_paths = run_plan
        .targets
        .iter()
        .map(|target| target.path.clone())
        .collect::<Vec<_>>();

    assert_eq!(dry_paths, run_paths);
    assert!(
        dry_plan
            .targets
            .iter()
            .all(|target| target.mode == DeleteMode::DryRun)
    );
    assert!(
        run_plan
            .targets
            .iter()
            .all(|target| target.mode == DeleteMode::RecycleBin)
    );
}

struct PlannerFixture {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    env: MapEnvironment,
}

impl PlannerFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let env = MapEnvironment::new()
            .with_var("TEMP", root.join("temp").into_os_string())
            .with_var("LOCALAPPDATA", root.join("local").into_os_string())
            .with_var("APPDATA", root.join("roaming").into_os_string())
            .with_var("USERPROFILE", root.join("user").into_os_string());

        Self {
            _temp: temp,
            root,
            env,
        }
    }

    fn with_local_temp_as_temp() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let env = MapEnvironment::new()
            .with_var("TEMP", root.join("local").join("Temp").into_os_string())
            .with_var("LOCALAPPDATA", root.join("local").into_os_string())
            .with_var("APPDATA", root.join("roaming").into_os_string())
            .with_var("USERPROFILE", root.join("user").into_os_string());

        Self {
            _temp: temp,
            root,
            env,
        }
    }

    fn with_cargo_home() -> Self {
        let fixture = Self::new();
        let env = fixture.env.clone().with_var(
            "CARGO_HOME",
            fixture.root.join("cargo-home").into_os_string(),
        );

        Self {
            _temp: fixture._temp,
            root: fixture.root,
            env,
        }
    }

    fn write(&self, relative: impl AsRef<Path>, bytes: &[u8]) {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }
}
