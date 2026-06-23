use std::fs;
use std::path::Path;

use rebecca_core::environment::MapEnvironment;
use rebecca_core::planner::build_cleanup_plan_with_environment;
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
            .with_var("APPDATA", root.join("roaming").into_os_string());

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
            .with_var("APPDATA", root.join("roaming").into_os_string());

        Self {
            _temp: temp,
            root,
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
