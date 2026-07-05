use std::fs;

use rebecca_core::RuleTargetSpec;
use rebecca_core::applications::{
    StaticApplicationDiscovery, SteamInstallation, parse_steam_libraryfolders,
};
use rebecca_core::discovery::{
    DiscoveryIndex, TargetResolution, resolve_rule_target, resolve_rule_target_with_applications,
    resolve_rule_target_with_applications_and_index,
};
use rebecca_core::environment::MapEnvironment;

#[test]
fn glob_template_discovers_profile_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("Profiles").join("alice").join("cache2")).unwrap();
    fs::create_dir_all(root.join("Profiles").join("bob").join("cache2")).unwrap();
    fs::create_dir_all(root.join("Profiles").join("carol").join("other")).unwrap();

    let env = MapEnvironment::new().with_var("ROOT", root.as_os_str().to_os_string());
    let target = RuleTargetSpec::glob_template("%ROOT%\\Profiles\\*\\cache2");

    let paths = match resolve_rule_target(&target, &env).unwrap() {
        TargetResolution::Paths(paths) => paths,
        TargetResolution::Skipped(reason) => panic!("target should resolve: {reason}"),
    };

    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&root.join("Profiles").join("alice").join("cache2")));
    assert!(paths.contains(&root.join("Profiles").join("bob").join("cache2")));
}

#[test]
fn glob_template_discovers_matching_files() {
    let temp = tempfile::tempdir().unwrap();
    let explorer = temp.path().join("Explorer");
    fs::create_dir_all(&explorer).unwrap();
    fs::write(explorer.join("thumbcache_96.db"), b"thumb").unwrap();
    fs::write(explorer.join("thumbcache_256.db"), b"thumb").unwrap();
    fs::write(explorer.join("not-a-cache.txt"), b"other").unwrap();

    let env = MapEnvironment::new().with_var("ROOT", temp.path().as_os_str().to_os_string());
    let target = RuleTargetSpec::glob_template("%ROOT%\\Explorer\\thumbcache_*.db");

    let paths = match resolve_rule_target(&target, &env).unwrap() {
        TargetResolution::Paths(paths) => paths,
        TargetResolution::Skipped(reason) => panic!("target should resolve: {reason}"),
    };

    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&explorer.join("thumbcache_96.db")));
    assert!(paths.contains(&explorer.join("thumbcache_256.db")));
}

#[test]
fn glob_template_with_no_matches_is_skipped() {
    let temp = tempfile::tempdir().unwrap();
    let env = MapEnvironment::new().with_var("ROOT", temp.path().as_os_str().to_os_string());
    let target = RuleTargetSpec::glob_template("%ROOT%\\missing\\*\\cache2");

    let resolution = resolve_rule_target(&target, &env).unwrap();

    assert_eq!(
        resolution,
        TargetResolution::Skipped("glob pattern matched no existing paths".to_string())
    );
}

#[test]
fn glob_template_does_not_traverse_symlinked_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let profiles = root.join("Profiles");
    let outside = root.join("OutsideProfile");
    fs::create_dir_all(profiles.join("alice").join("cache2")).unwrap();
    fs::create_dir_all(outside.join("cache2")).unwrap();
    create_dir_symlink(&outside, &profiles.join("linked")).unwrap();

    let env = MapEnvironment::new().with_var("ROOT", root.as_os_str().to_os_string());
    let target = RuleTargetSpec::glob_template("%ROOT%\\Profiles\\*\\cache2");

    let paths = match resolve_rule_target(&target, &env).unwrap() {
        TargetResolution::Paths(paths) => paths,
        TargetResolution::Skipped(reason) => panic!("target should resolve: {reason}"),
    };

    assert_eq!(paths, vec![profiles.join("alice").join("cache2")]);
}

#[test]
fn glob_template_shared_index_reuses_compatible_directory_listing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("Profiles").join("alice").join("cache2")).unwrap();
    fs::create_dir_all(root.join("Profiles").join("alice").join("startupCache")).unwrap();
    fs::create_dir_all(root.join("Profiles").join("bob").join("cache2")).unwrap();
    fs::create_dir_all(root.join("Profiles").join("bob").join("startupCache")).unwrap();

    let env = MapEnvironment::new().with_var("ROOT", root.as_os_str().to_os_string());
    let applications = StaticApplicationDiscovery::new();
    let mut index = DiscoveryIndex::new();
    let cache_target = RuleTargetSpec::glob_template("%ROOT%\\Profiles\\*\\cache2");
    let startup_target = RuleTargetSpec::glob_template("%ROOT%\\Profiles\\*\\startupCache");

    let first = resolve_rule_target_with_applications_and_index(
        &cache_target,
        &env,
        &applications,
        &mut index,
    )
    .unwrap();
    let cached_after_first = index.cached_glob_directory_count();
    let second = resolve_rule_target_with_applications_and_index(
        &startup_target,
        &env,
        &applications,
        &mut index,
    )
    .unwrap();

    assert!(matches!(first, TargetResolution::Paths(paths) if paths.len() == 2));
    assert!(matches!(second, TargetResolution::Paths(paths) if paths.len() == 2));
    assert_eq!(cached_after_first, index.cached_glob_directory_count());
}

#[test]
fn exact_search_semantics_do_not_use_glob_index() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("Profiles").join("alice").join("cache2")).unwrap();

    let env = MapEnvironment::new().with_var("ROOT", root.as_os_str().to_os_string());
    let applications = StaticApplicationDiscovery::new();
    let mut index = DiscoveryIndex::new();
    let glob_target = RuleTargetSpec::glob_template("%ROOT%\\Profiles\\*\\cache2");
    let exact_target = RuleTargetSpec::template("%ROOT%\\Profiles\\alice\\cache2");

    resolve_rule_target_with_applications_and_index(&glob_target, &env, &applications, &mut index)
        .unwrap();
    let cached_after_glob = index.cached_glob_directory_count();
    resolve_rule_target_with_applications_and_index(&exact_target, &env, &applications, &mut index)
        .unwrap();

    assert_eq!(cached_after_glob, index.cached_glob_directory_count());
}

#[test]
fn steam_install_template_expands_from_discovered_install_path() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let steam = SteamInstallation::new(install_path.clone(), Vec::<std::path::PathBuf>::new());
    let applications = StaticApplicationDiscovery::new().with_steam_installation(steam);
    let env = MapEnvironment::new();
    let target = RuleTargetSpec::steam_install_template("appcache\\httpcache");

    let paths = match resolve_rule_target_with_applications(&target, &env, &applications).unwrap() {
        TargetResolution::Paths(paths) => paths,
        TargetResolution::Skipped(reason) => panic!("target should resolve: {reason}"),
    };

    assert_eq!(paths, vec![install_path.join("appcache").join("httpcache")]);
}

#[test]
fn steam_install_template_skips_when_steam_is_not_discovered() {
    let applications = StaticApplicationDiscovery::new();
    let env = MapEnvironment::new();
    let target = RuleTargetSpec::steam_install_template("appcache\\httpcache");

    let resolution = resolve_rule_target_with_applications(&target, &env, &applications).unwrap();

    assert_eq!(
        resolution,
        TargetResolution::Skipped("Steam installation was not discovered".to_string())
    );
}

#[test]
fn steam_installation_reads_libraryfolders_from_install_path() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let steamapps = install_path.join("steamapps");
    fs::create_dir_all(&steamapps).unwrap();
    fs::write(
        steamapps.join("libraryfolders.vdf"),
        r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
    }
    "1" "D:\\SteamLibrary"
}
"#,
    )
    .unwrap();

    let installation = SteamInstallation::from_install_path(&install_path).unwrap();

    assert_eq!(installation.install_path(), install_path.as_path());
    assert_eq!(installation.library_paths().len(), 2);
    assert!(
        installation
            .library_paths()
            .contains(&std::path::PathBuf::from(r"C:\Program Files (x86)\Steam"))
    );
    assert!(
        installation
            .library_paths()
            .contains(&std::path::PathBuf::from(r"D:\SteamLibrary"))
    );
}

#[test]
fn steam_installation_merges_config_and_steamapps_libraryfolders() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let config = install_path.join("config");
    let steamapps = install_path.join("steamapps");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&steamapps).unwrap();
    fs::write(
        config.join("libraryfolders.vdf"),
        r#"
"libraryfolders"
{
    "0"
    {
        "path"      "D:\\ConfigLibrary"
    }
}
"#,
    )
    .unwrap();
    fs::write(
        steamapps.join("libraryfolders.vdf"),
        r#"
"libraryfolders"
{
    "1"
    {
        "path"      "E:\\SteamAppsLibrary"
    }
}
"#,
    )
    .unwrap();

    let installation = SteamInstallation::from_install_path(&install_path).unwrap();

    assert_eq!(installation.install_path(), install_path.as_path());
    assert_eq!(
        installation.library_paths(),
        &[
            std::path::PathBuf::from(r"D:\ConfigLibrary"),
            std::path::PathBuf::from(r"E:\SteamAppsLibrary")
        ]
    );
}

#[test]
fn steam_installation_ignores_missing_but_keeps_readable_libraryfolders() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let config = install_path.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("libraryfolders.vdf"),
        r#"
"libraryfolders"
{
    "0"
    {
        "path"      "D:\\ConfigLibrary"
    }
}
"#,
    )
    .unwrap();

    let installation = SteamInstallation::from_install_path(&install_path).unwrap();

    assert_eq!(installation.install_path(), install_path.as_path());
    assert_eq!(
        installation.library_paths(),
        &[std::path::PathBuf::from(r"D:\ConfigLibrary")]
    );
}

#[test]
fn steam_installation_reports_read_errors_for_libraryfolders_file() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let library_file = install_path.join("steamapps").join("libraryfolders.vdf");
    fs::create_dir_all(&library_file).unwrap();

    let err = SteamInstallation::from_install_path(&install_path).unwrap_err();

    assert!(
        err.to_string()
            .contains("could not read Steam library folders")
    );
}

#[test]
fn steam_installation_reports_malformed_libraryfolders_vdf() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let steamapps = install_path.join("steamapps");
    fs::create_dir_all(&steamapps).unwrap();
    fs::write(steamapps.join("libraryfolders.vdf"), "\"libraryfolders").unwrap();

    let err = SteamInstallation::from_install_path(&install_path).unwrap_err();

    assert!(
        err.to_string()
            .contains("unterminated string in Steam libraryfolders.vdf")
    );
}

#[test]
fn steam_installation_treats_missing_libraryfolders_as_empty_library_paths() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    fs::create_dir_all(install_path.join("steamapps")).unwrap();

    let installation = SteamInstallation::from_install_path(&install_path).unwrap();

    assert_eq!(installation.install_path(), install_path.as_path());
    assert!(installation.library_paths().is_empty());
}

#[test]
fn steam_installation_deduplicates_install_path_from_library_paths() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let library_path = temp.path().join("SteamLibrary");
    let installation = SteamInstallation::new(
        install_path.clone(),
        vec![install_path.clone(), library_path.clone()],
    );

    assert_eq!(installation.install_path(), install_path.as_path());
    assert_eq!(installation.library_paths(), &[library_path]);
}

#[test]
fn steam_installation_deduplicates_install_path_case_insensitively() {
    let install_path = std::path::PathBuf::from(r"C:\Steam");
    let library_path = std::path::PathBuf::from(r"c:\steam");
    let installation = SteamInstallation::new(install_path.clone(), vec![library_path]);

    assert_eq!(installation.install_path(), install_path.as_path());
    assert!(installation.library_paths().is_empty());
}

#[test]
fn steam_installation_deduplicates_trailing_separator_variants() {
    let install_path = std::path::PathBuf::from(r"C:\Steam");
    let library_path = std::path::PathBuf::from(r"C:\Steam\\");
    let installation = SteamInstallation::new(install_path.clone(), vec![library_path]);

    assert_eq!(installation.install_path(), install_path.as_path());
    assert!(installation.library_paths().is_empty());
}

#[test]
fn steam_libraryfolders_parser_deduplicates_trailing_separator_variants() {
    let raw = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\SteamLibrary\\"
    }
    "1"
    {
        "path"      "c:/steamlibrary"
    }
}
"#;

    let paths = parse_steam_libraryfolders(raw).unwrap();

    assert_eq!(paths, vec![std::path::PathBuf::from(r"C:\SteamLibrary\\")]);
}

#[test]
fn steam_libraryfolders_parser_ignores_relative_paths() {
    let raw = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "SteamLibrary"
    }
    "1" "D:\\SteamLibrary"
}
"#;

    let paths = parse_steam_libraryfolders(raw).unwrap();

    assert_eq!(paths, vec![std::path::PathBuf::from(r"D:\SteamLibrary")]);
}

#[test]
fn steam_library_template_expands_all_discovered_library_paths() {
    let temp = tempfile::tempdir().unwrap();
    let install_path = temp.path().join("Steam");
    let library_path = temp.path().join("SteamLibrary");
    let steam = SteamInstallation::new(install_path.clone(), vec![library_path.clone()]);
    let applications = StaticApplicationDiscovery::new().with_steam_installation(steam);
    let env = MapEnvironment::new();
    let target = RuleTargetSpec::steam_library_template("steamapps\\shadercache");

    let paths = match resolve_rule_target_with_applications(&target, &env, &applications).unwrap() {
        TargetResolution::Paths(paths) => paths,
        TargetResolution::Skipped(reason) => panic!("target should resolve: {reason}"),
    };

    assert_eq!(
        paths,
        vec![
            install_path.join("steamapps").join("shadercache"),
            library_path.join("steamapps").join("shadercache")
        ]
    );
}

#[test]
fn steam_templates_skip_when_steam_is_not_discovered() {
    let applications = StaticApplicationDiscovery::new();
    let env = MapEnvironment::new();
    let target = RuleTargetSpec::steam_library_template("steamapps\\shadercache");

    let resolution = resolve_rule_target_with_applications(&target, &env, &applications).unwrap();

    assert_eq!(
        resolution,
        TargetResolution::Skipped("Steam installation was not discovered".to_string())
    );
}

#[test]
fn steam_relative_templates_reject_absolute_or_parent_paths() {
    let temp = tempfile::tempdir().unwrap();
    let steam = SteamInstallation::new(temp.path().join("Steam"), Vec::<std::path::PathBuf>::new());
    let applications = StaticApplicationDiscovery::new().with_steam_installation(steam);
    let env = MapEnvironment::new();
    let target = RuleTargetSpec::steam_install_template("..\\userdata");

    let err = resolve_rule_target_with_applications(&target, &env, &applications).unwrap_err();

    assert!(err.to_string().contains("must be a safe relative path"));
}

#[test]
fn steam_relative_templates_reject_current_directory_segments() {
    let temp = tempfile::tempdir().unwrap();
    let steam = SteamInstallation::new(temp.path().join("Steam"), Vec::<std::path::PathBuf>::new());
    let applications = StaticApplicationDiscovery::new().with_steam_installation(steam);
    let env = MapEnvironment::new();
    let target = RuleTargetSpec::steam_library_template(".\\shadercache");

    let err = resolve_rule_target_with_applications(&target, &env, &applications).unwrap_err();

    assert!(err.to_string().contains("must be a safe relative path"));
}

#[cfg(unix)]
fn create_dir_symlink(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_dir_symlink(target: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[test]
fn steam_libraryfolders_parser_supports_current_nested_format() {
    let raw = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
        "apps"
        {
            "228980" "492988589"
        }
    }
    "1"
    {
        "path"      "E:\\SteamLibrary"
    }
}
"#;

    let paths = parse_steam_libraryfolders(raw).unwrap();

    assert_eq!(
        paths,
        vec![
            std::path::PathBuf::from(r"C:\Program Files (x86)\Steam"),
            std::path::PathBuf::from(r"E:\SteamLibrary")
        ]
    );
}

#[test]
fn steam_libraryfolders_parser_supports_legacy_flat_format() {
    let raw = r#"
"LibraryFolders"
{
    "TimeNextStatsReport" "123"
    "1" "D:\\SteamLibrary"
    "2" "E:\\SteamLibrary"
}
"#;

    let paths = parse_steam_libraryfolders(raw).unwrap();

    assert_eq!(
        paths,
        vec![
            std::path::PathBuf::from(r"D:\SteamLibrary"),
            std::path::PathBuf::from(r"E:\SteamLibrary")
        ]
    );
}

#[test]
fn steam_libraryfolders_parser_deduplicates_case_insensitive_paths() {
    let raw = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\SteamLibrary"
    }
    "1"
    {
        "path"      "c:/steamlibrary"
    }
}
"#;

    let paths = parse_steam_libraryfolders(raw).unwrap();

    assert_eq!(paths, vec![std::path::PathBuf::from(r"C:\SteamLibrary")]);
}
