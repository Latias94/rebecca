use std::fs;
use std::path::Path;

use rebecca_core::applications::NoopApplicationDiscovery;
use rebecca_core::environment::SystemEnvironment;
use rebecca_core::plan::CleanupTargetIssueReason;
use rebecca_core::planner::{PlanBuildContext, build_cleanup_plan_with_context};
use rebecca_core::project_artifacts::{ProjectArtifactScanOptions, discover_project_artifacts};
use rebecca_core::scan::ScanCancellationToken;
use rebecca_core::{CleanupWorkflow, DeleteMode, PlanRequest, Platform, TargetStatus};

const CACHEDIR_TAG_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn write_cachedir_tag(dir: impl AsRef<Path>) {
    write_fixture_file(
        dir.as_ref().join("CACHEDIR.TAG"),
        format!("{CACHEDIR_TAG_SIGNATURE}\n# cache directory\n").as_bytes(),
    );
}

#[test]
fn discovers_known_project_artifacts_and_prunes_nested_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_fixture_file(workspace.join("app").join("package.json"), b"{}");
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("node_modules")
            .join("nested.bin"),
        b"nested",
    );
    write_fixture_file(workspace.join("app").join("Cargo.toml"), b"[package]");
    write_fixture_file(workspace.join("app").join("target.txt"), b"keep");
    write_fixture_file(
        workspace.join("app").join("vendor").join("dep.bin"),
        b"keep",
    );

    let options = ProjectArtifactScanOptions::new(vec![workspace]).with_max_depth(4);
    let artifacts = discover_project_artifacts(&options, &ScanCancellationToken::new()).unwrap();
    let paths = artifacts
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect::<Vec<_>>();

    assert_eq!(paths.len(), 2);
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with(Path::new("app").join("node_modules")))
    );
    assert!(
        paths
            .iter()
            .any(|path| path.ends_with(Path::new("app").join("target")))
    );
    assert!(
        paths
            .iter()
            .all(|path| !path.ends_with(Path::new("target").join("node_modules")))
    );

    let node_modules = artifacts
        .iter()
        .find(|artifact| {
            artifact
                .path
                .ends_with(Path::new("app").join("node_modules"))
        })
        .unwrap();
    assert_eq!(node_modules.context.matched_context, "node-project");
    assert!(node_modules.context.project_root.ends_with("app"));
    assert!(
        node_modules
            .context
            .project_anchor
            .ends_with(Path::new("app").join("package.json"))
    );

    let target = artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with(Path::new("app").join("target")))
        .unwrap();
    assert_eq!(target.context.matched_context, "target-project");
    assert!(
        target
            .context
            .project_anchor
            .ends_with(Path::new("app").join("Cargo.toml"))
    );
}

#[test]
fn skips_embedded_toolchain_artifacts_without_project_context() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let unity_package_manager_node_modules = workspace
        .join("Game Engines")
        .join("Unity Editors")
        .join("2021.3.13f1")
        .join("Editor")
        .join("Data")
        .join("Resources")
        .join("PackageManager")
        .join("Server")
        .join("node_modules");
    let unity_nodejs_node_modules = workspace
        .join("Game Engines")
        .join("Unity Editors")
        .join("2021.3.13f1")
        .join("Editor")
        .join("Data")
        .join("Tools")
        .join("nodejs")
        .join("node_modules");
    let installed_rust_target = workspace
        .join("SDKs")
        .join("rust-toolchain")
        .join("lib")
        .join("target");
    let nested_build_under_embedded_node_modules = unity_package_manager_node_modules
        .join("@edt")
        .join("proxy-helper")
        .join("build");
    let nested_dist_under_embedded_node_modules = unity_package_manager_node_modules
        .join("logform")
        .join("dist");
    let nested_node_modules_under_embedded_node_modules =
        unity_nodejs_node_modules.join("npm").join("node_modules");
    let embedded_python_cache = workspace
        .join("Game Engines")
        .join("Unity Editors")
        .join("2021.3.13f1")
        .join("Editor")
        .join("Data")
        .join("PlaybackEngines")
        .join("AndroidPlayer")
        .join("NDK")
        .join("python-packages")
        .join("adb")
        .join("__pycache__");
    let real_node_modules = workspace
        .join("SourceCodes")
        .join("web-app")
        .join("node_modules");
    let real_target = workspace
        .join("SourceCodes")
        .join("rust-app")
        .join("target");

    write_fixture_file(unity_package_manager_node_modules.join("pkg.bin"), b"unity");
    write_fixture_file(unity_nodejs_node_modules.join("npm.bin"), b"node");
    write_fixture_file(installed_rust_target.join("triple").join("lib.bin"), b"sdk");
    write_fixture_file(
        nested_build_under_embedded_node_modules.join("out.bin"),
        b"build",
    );
    write_fixture_file(
        nested_dist_under_embedded_node_modules.join("bundle.js"),
        b"dist",
    );
    write_fixture_file(
        nested_node_modules_under_embedded_node_modules
            .join("left-pad")
            .join("pkg.bin"),
        b"nested",
    );
    write_fixture_file(embedded_python_cache.join("adb.pyc"), b"pycache");
    write_fixture_file(
        workspace
            .join("Game Engines")
            .join("Unity Editors")
            .join("2021.3.13f1")
            .join("Editor")
            .join("Data")
            .join("PlaybackEngines")
            .join("AndroidPlayer")
            .join("NDK")
            .join("python-packages")
            .join("adb")
            .join("setup.py"),
        b"from setuptools import setup",
    );
    write_fixture_file(real_node_modules.join("pkg.bin"), b"web");
    write_fixture_file(
        workspace
            .join("SourceCodes")
            .join("web-app")
            .join("package.json"),
        b"{}",
    );
    write_fixture_file(real_target.join("debug").join("app.bin"), b"rust");
    write_fixture_file(
        workspace
            .join("SourceCodes")
            .join("rust-app")
            .join("Cargo.toml"),
        b"[package]",
    );

    let options = ProjectArtifactScanOptions::new(vec![workspace]).with_max_depth(12);
    let artifacts = discover_project_artifacts(&options, &ScanCancellationToken::new()).unwrap();

    assert!(artifacts.iter().any(|artifact| {
        artifact.definition.rule_id == "windows.project-artifact-node-modules"
            && artifact.path == real_node_modules
    }));
    assert!(artifacts.iter().any(|artifact| {
        artifact.definition.rule_id == "windows.project-artifact-target"
            && artifact.path == real_target
    }));
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == unity_package_manager_node_modules)
    );
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == nested_build_under_embedded_node_modules)
    );
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == nested_dist_under_embedded_node_modules)
    );
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == unity_nodejs_node_modules)
    );
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == nested_node_modules_under_embedded_node_modules)
    );
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == installed_rust_target)
    );
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == embedded_python_cache)
    );
}

#[test]
fn discovers_valid_cachedir_tag_directories() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let cache = workspace.join("app").join("custom-cache");
    let invalid_cache = workspace.join("app").join("invalid-cache");
    write_fixture_file(cache.join("entry.bin"), b"abc");
    write_cachedir_tag(&cache);
    write_fixture_file(invalid_cache.join("entry.bin"), b"keep");
    write_fixture_file(
        invalid_cache.join("CACHEDIR.TAG"),
        b"not the standard signature",
    );
    write_cachedir_tag(&workspace);

    let options = ProjectArtifactScanOptions::new(vec![workspace.clone()]).with_max_depth(4);
    let artifacts = discover_project_artifacts(&options, &ScanCancellationToken::new()).unwrap();

    assert_eq!(artifacts.len(), 1);
    assert_eq!(
        artifacts[0].definition.rule_id,
        "windows.project-artifact-cachedir-tag"
    );
    assert_eq!(artifacts[0].path, cache);
    assert_ne!(artifacts[0].path, workspace);
}

#[test]
fn discovers_context_sensitive_bin_and_vendor_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let dotnet_bin = workspace.join("dotnet-app").join("bin");
    let composer_vendor = workspace.join("php-app").join("vendor");
    let generic_bin = workspace.join("generic-tool").join("bin");
    let go_vendor = workspace.join("go-app").join("vendor");
    let rails_vendor = workspace.join("rails-app").join("vendor");
    let unknown_vendor = workspace.join("unknown-app").join("vendor");

    write_fixture_file(dotnet_bin.join("Debug").join("app.dll"), b"dotnet");
    write_fixture_file(
        workspace.join("dotnet-app").join("App.csproj"),
        b"<Project />",
    );
    write_fixture_file(generic_bin.join("Release").join("tool.exe"), b"generic");
    write_fixture_file(composer_vendor.join("pkg").join("autoload.php"), b"php");
    write_fixture_file(workspace.join("php-app").join("composer.json"), b"{}");
    write_fixture_file(go_vendor.join("pkg").join("dep.go"), b"go");
    write_fixture_file(workspace.join("go-app").join("go.mod"), b"module example");
    write_fixture_file(rails_vendor.join("javascript").join("dep.js"), b"rails");
    write_fixture_file(workspace.join("rails-app").join("Gemfile"), b"source");
    write_fixture_file(unknown_vendor.join("dep.bin"), b"unknown");

    let options = ProjectArtifactScanOptions::new(vec![workspace]).with_max_depth(4);
    let artifacts = discover_project_artifacts(&options, &ScanCancellationToken::new()).unwrap();

    assert!(artifacts.iter().any(|artifact| {
        artifact.definition.rule_id == "windows.project-artifact-dotnet-bin"
            && artifact.path == dotnet_bin
    }));
    assert!(artifacts.iter().any(|artifact| {
        artifact.definition.rule_id == "windows.project-artifact-composer-vendor"
            && artifact.path == composer_vendor
    }));
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == generic_bin)
    );
    assert!(!artifacts.iter().any(|artifact| artifact.path == go_vendor));
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == rails_vendor)
    );
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == unknown_vendor)
    );
}

#[test]
fn discovers_context_sensitive_build_dist_and_obj_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let rust_build = workspace.join("rust-app").join("build");
    let js_dist = workspace.join("web-app").join("dist");
    let dotnet_obj = workspace.join("dotnet-app").join("obj");
    let engine_build = workspace
        .join("Epic Games")
        .join("UE_5.3")
        .join("Engine")
        .join("Intermediate")
        .join("Build");
    let sdk_obj = workspace.join("SDK").join("toolchain").join("obj");
    let generic_dist = workspace.join("downloads").join("dist");

    write_fixture_file(rust_build.join("out.bin"), b"rust");
    write_fixture_file(workspace.join("rust-app").join("Cargo.toml"), b"[package]");
    write_fixture_file(js_dist.join("bundle.js"), b"web");
    write_fixture_file(workspace.join("web-app").join("package.json"), b"{}");
    write_fixture_file(dotnet_obj.join("Debug").join("app.obj"), b"dotnet");
    write_fixture_file(
        workspace.join("dotnet-app").join("App.csproj"),
        b"<Project />",
    );
    write_fixture_file(engine_build.join("receipt.bin"), b"engine");
    write_fixture_file(sdk_obj.join("tool.obj"), b"sdk");
    write_fixture_file(generic_dist.join("archive.zip"), b"download");

    let options = ProjectArtifactScanOptions::new(vec![workspace]).with_max_depth(6);
    let artifacts = discover_project_artifacts(&options, &ScanCancellationToken::new()).unwrap();

    assert!(artifacts.iter().any(|artifact| {
        artifact.definition.rule_id == "windows.project-artifact-build"
            && artifact.path == rust_build
    }));
    assert!(artifacts.iter().any(|artifact| {
        artifact.definition.rule_id == "windows.project-artifact-dist" && artifact.path == js_dist
    }));
    assert!(artifacts.iter().any(|artifact| {
        artifact.definition.rule_id == "windows.project-artifact-dotnet-obj"
            && artifact.path == dotnet_obj
    }));
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == engine_build)
    );
    assert!(!artifacts.iter().any(|artifact| artifact.path == sdk_obj));
    assert!(
        !artifacts
            .iter()
            .any(|artifact| artifact.path == generic_dist)
    );
}

#[test]
fn project_artifact_scan_respects_max_depth() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace
            .join("level1")
            .join("level2")
            .join("node_modules")
            .join("pkg.bin"),
        b"abc",
    );
    write_fixture_file(
        workspace.join("level1").join("level2").join("package.json"),
        b"{}",
    );

    let shallow = ProjectArtifactScanOptions::new(vec![workspace.clone()]).with_max_depth(1);
    assert!(
        discover_project_artifacts(&shallow, &ScanCancellationToken::new())
            .unwrap()
            .is_empty()
    );

    let deep = ProjectArtifactScanOptions::new(vec![workspace]).with_max_depth(3);
    assert_eq!(
        discover_project_artifacts(&deep, &ScanCancellationToken::new())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn project_artifact_plan_measures_allowed_targets_and_blocks_user_protected_paths() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let node_modules = workspace.join("app").join("node_modules");
    let target = workspace.join("app").join("target");
    write_fixture_file(node_modules.join("pkg.bin"), b"abc");
    write_fixture_file(target.join("debug").join("app.bin"), b"blocked");
    write_fixture_file(workspace.join("app").join("package.json"), b"{}");
    write_fixture_file(workspace.join("app").join("Cargo.toml"), b"[package]");

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = vec![workspace];
    request.project_artifact_max_depth = 4;
    request.project_artifact_min_age_days = 0;
    let protected_paths = vec![target.clone()];
    let cancellation = ScanCancellationToken::new();
    let applications = NoopApplicationDiscovery::new();

    let plan = build_cleanup_plan_with_context(
        &request,
        &[],
        &SystemEnvironment,
        &applications,
        PlanBuildContext::new(&cancellation).with_protected_paths(&protected_paths),
        |_| {},
    )
    .unwrap();

    assert_eq!(plan.summary.total_targets, 2);
    assert_eq!(plan.summary.allowed_targets, 1);
    assert_eq!(plan.summary.blocked_targets, 1);
    assert_eq!(plan.summary.estimated_bytes, 3);

    let allowed = plan
        .targets
        .iter()
        .find(|target| target.status == TargetStatus::Allowed)
        .unwrap();
    assert_eq!(allowed.rule_id, "windows.project-artifact-node-modules");
    assert!(allowed.restore_hint.is_some());

    let blocked = plan
        .targets
        .iter()
        .find(|target| target.status == TargetStatus::Blocked)
        .unwrap();
    assert_eq!(blocked.path, target);
    assert_eq!(
        blocked.reason_code,
        Some(CleanupTargetIssueReason::SafetyPolicyBlocked)
    );
    assert!(
        blocked
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("user-protected path"))
    );
}

#[test]
fn project_artifact_plan_filters_selected_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_fixture_file(workspace.join("app").join("package.json"), b"{}");
    write_fixture_file(
        workspace
            .join("app")
            .join("target")
            .join("debug")
            .join("app.bin"),
        b"rust",
    );
    write_fixture_file(workspace.join("app").join("Cargo.toml"), b"[package]");

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = vec![workspace];
    request.project_artifact_max_depth = 4;
    request.project_artifact_min_age_days = 0;
    request.project_artifact_selectors = vec!["node-modules".to_string()];
    let cancellation = ScanCancellationToken::new();
    let applications = NoopApplicationDiscovery::new();

    let plan = build_cleanup_plan_with_context(
        &request,
        &[],
        &SystemEnvironment,
        &applications,
        PlanBuildContext::new(&cancellation),
        |_| {},
    )
    .unwrap();

    assert_eq!(plan.summary.total_targets, 1);
    assert_eq!(plan.summary.allowed_targets, 1);
    assert_eq!(plan.summary.estimated_bytes, 3);
    assert_eq!(
        plan.targets[0].rule_id,
        "windows.project-artifact-node-modules"
    );
}

#[test]
fn project_artifact_plan_rejects_unknown_artifact_selectors() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).unwrap();

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = vec![workspace];
    request.project_artifact_selectors = vec!["missing-artifact".to_string()];
    let cancellation = ScanCancellationToken::new();
    let applications = NoopApplicationDiscovery::new();

    let err = build_cleanup_plan_with_context(
        &request,
        &[],
        &SystemEnvironment,
        &applications,
        PlanBuildContext::new(&cancellation),
        |_| {},
    )
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("invalid project artifact selector")
    );
    assert!(err.to_string().contains("missing-artifact"));
}

#[test]
fn project_artifact_plan_skips_recent_targets_before_sizing() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_fixture_file(workspace.join("app").join("package.json"), b"{}");

    let mut request = PlanRequest::for_platform(Platform::Windows, DeleteMode::DryRun)
        .with_workflow(CleanupWorkflow::ProjectArtifacts);
    request.project_artifact_roots = vec![workspace];
    request.project_artifact_max_depth = 4;
    let cancellation = ScanCancellationToken::new();
    let applications = NoopApplicationDiscovery::new();

    let plan = build_cleanup_plan_with_context(
        &request,
        &[],
        &SystemEnvironment,
        &applications,
        PlanBuildContext::new(&cancellation),
        |_| {},
    )
    .unwrap();

    assert_eq!(plan.summary.total_targets, 1);
    assert_eq!(plan.summary.allowed_targets, 0);
    assert_eq!(plan.summary.skipped_targets, 1);
    assert_eq!(plan.summary.estimated_bytes, 0);
    assert_eq!(plan.targets[0].status, TargetStatus::Skipped);
    assert_eq!(
        plan.targets[0].reason_code,
        Some(CleanupTargetIssueReason::ProjectArtifactRecentlyModified)
    );
    assert!(
        plan.targets[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("modified within the last 7 days"))
    );
}
