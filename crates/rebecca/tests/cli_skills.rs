mod common;

const REPO_SKILL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../skills/rebecca-disk-cleaner/SKILL.md"
);
const PACKAGED_SKILL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/skills/rebecca-disk-cleaner/SKILL.md"
);
const API_PAYLOADS_SCHEMA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/api/cli/v1/payloads.schema.json"
);

fn skill_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join("rebecca-disk-cleaner")
}

fn normalized_text(path: &str) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

fn validator_for_payload_def(def_name: &str) -> jsonschema::Validator {
    let payloads: serde_json::Value =
        serde_json::from_slice(&std::fs::read(API_PAYLOADS_SCHEMA).unwrap()).unwrap();
    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$defs": payloads["$defs"].clone(),
        "$ref": format!("#/$defs/{def_name}"),
    });
    jsonschema::validator_for(&schema).unwrap()
}

#[test]
fn packaged_skill_matches_repository_skill() {
    assert_eq!(normalized_text(PACKAGED_SKILL), normalized_text(REPO_SKILL));
}

#[test]
fn skills_path_defaults_to_agents_skills_root() {
    let temp = tempfile::tempdir().unwrap();
    let output = common::isolated::isolated_rebecca(&temp)
        .args(["skills", "path", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "skills path");
    assert_eq!(envelope["payload_kind"], "skill-management");
    assert_eq!(envelope["data"]["skill"], "rebecca-disk-cleaner");
    assert_eq!(envelope["data"]["agent"], "agents");
    assert_eq!(envelope["data"]["status"], "path");
    assert_eq!(envelope["data"]["managed"], false);
    assert!(
        envelope["data"]["skills_dir"]
            .as_str()
            .unwrap()
            .ends_with("/.agents/skills")
    );
}

#[test]
fn skills_path_supports_codex_home_preset() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex-home");
    let output = common::isolated::isolated_rebecca(&temp)
        .env("CODEX_HOME", &codex_home)
        .args(["skills", "path", "--agent", "codex", "--format", "json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["data"]["agent"], "codex");
    assert!(
        envelope["data"]["skills_dir"]
            .as_str()
            .unwrap()
            .ends_with("/codex-home/skills")
    );
}

#[test]
fn skills_install_dry_run_reports_without_writing() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("custom-skills");
    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "install",
            "--destination",
            destination.to_str().unwrap(),
            "--dry-run",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(!destination.exists());

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "skills install");
    assert_eq!(envelope["data"]["agent"], "custom");
    assert_eq!(envelope["data"]["status"], "would-install");
    assert_eq!(envelope["data"]["dry_run"], true);
    assert_eq!(envelope["data"]["changed"], false);
    validator_for_payload_def("skillManagement")
        .validate(&envelope["data"])
        .unwrap();
}

#[test]
fn skills_install_writes_packaged_skill_and_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("custom-skills");
    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "install",
            "--destination",
            destination.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let installed = skill_dir(&destination);
    assert_eq!(
        std::fs::read(installed.join("SKILL.md")).unwrap(),
        std::fs::read(PACKAGED_SKILL).unwrap()
    );
    assert!(installed.join(".rebecca-skill.json").is_file());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Status: installed"));
}

#[test]
fn skills_install_is_idempotent_when_content_matches() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("custom-skills");
    for _ in 0..2 {
        let output = common::isolated::isolated_rebecca(&temp)
            .args([
                "skills",
                "install",
                "--destination",
                destination.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "stderr: {}",
            common::support::stderr(&output)
        );
    }

    let output = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "install",
            "--destination",
            destination.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["data"]["status"], "unchanged");
    assert_eq!(envelope["data"]["changed"], false);
}

#[test]
fn skills_install_requires_force_to_replace_different_content() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("custom-skills");
    let installed = skill_dir(&destination);
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(
        installed.join("SKILL.md"),
        "---\nname: rebecca-disk-cleaner\n---\nlocal edit\n",
    )
    .unwrap();

    let rejected = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "install",
            "--destination",
            destination.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(common::support::stderr(&rejected).contains("pass --force to replace it"));

    let replaced = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "install",
            "--destination",
            destination.to_str().unwrap(),
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        replaced.status.success(),
        "stderr: {}",
        common::support::stderr(&replaced)
    );
    assert_eq!(
        std::fs::read(installed.join("SKILL.md")).unwrap(),
        std::fs::read(PACKAGED_SKILL).unwrap()
    );
}

#[test]
fn skills_delete_alias_removes_managed_skill() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("custom-skills");
    let installed = skill_dir(&destination);

    let install = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "install",
            "--destination",
            destination.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        install.status.success(),
        "stderr: {}",
        common::support::stderr(&install)
    );
    assert!(installed.is_dir());

    let remove = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "delete",
            "--destination",
            destination.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(
        remove.status.success(),
        "stderr: {}",
        common::support::stderr(&remove)
    );
    assert!(!installed.exists());
    let envelope = common::support::api_envelope(&remove.stdout);
    assert_eq!(envelope["command"], "skills remove");
    assert_eq!(envelope["data"]["status"], "removed");
    assert_eq!(envelope["data"]["changed"], true);
}

#[test]
fn skills_remove_refuses_unrecognized_directory_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("custom-skills");
    let installed = skill_dir(&destination);
    std::fs::create_dir_all(&installed).unwrap();
    std::fs::write(installed.join("SKILL.md"), "---\nname: other\n---\n").unwrap();

    let rejected = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "remove",
            "--destination",
            destination.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(installed.exists());
    assert!(
        common::support::stderr(&rejected).contains("does not look like a Rebecca-managed skill")
    );

    let forced = common::isolated::isolated_rebecca(&temp)
        .args([
            "skills",
            "remove",
            "--destination",
            destination.to_str().unwrap(),
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        forced.status.success(),
        "stderr: {}",
        common::support::stderr(&forced)
    );
    assert!(!installed.exists());
}
