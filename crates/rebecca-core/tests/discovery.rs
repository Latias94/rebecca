use std::fs;

use rebecca_core::RuleTargetSpec;
use rebecca_core::discovery::{TargetResolution, resolve_rule_target};
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
