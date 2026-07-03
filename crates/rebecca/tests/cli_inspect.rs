use std::fs;
use std::path::{Path, PathBuf};

mod common;
#[path = "common/isolated.rs"]
mod isolated;

fn write_fixture_file(path: impl AsRef<Path>, bytes: &[u8]) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

fn write_node_project(dir: impl AsRef<Path>) {
    write_fixture_file(dir.as_ref().join("package.json"), b"{}");
}

fn write_rust_project(dir: impl AsRef<Path>) {
    write_fixture_file(dir.as_ref().join("Cargo.toml"), b"[package]");
}

#[test]
fn inspect_help_lists_space_and_artifacts_subcommands() {
    let output = common::command::rebecca()
        .args(["inspect", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("space"));
    assert!(stdout.contains("map"));
    assert!(stdout.contains("artifacts"));
    assert!(stdout.contains("lint"));
}

#[test]
fn inspect_space_json_reports_top_entries_and_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let missing = temp.path().join("missing");
    write_fixture_file(root.join("zeta").join("data.bin"), b"abc");
    write_fixture_file(root.join("alpha").join("data.bin"), b"abc");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "space",
            "--format",
            "json",
            "--root",
            root.to_str().unwrap(),
            "--root",
            missing.to_str().unwrap(),
            "--top",
            "2",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "inspect space");
    assert_eq!(envelope["payload_kind"], "inspect-space");

    let value = &envelope["data"];
    assert_eq!(value["totals"]["estimated_bytes"], 7);
    assert_eq!(value["totals"]["files"], 3);
    assert_eq!(value["top_entries"].as_array().unwrap().len(), 2);
    assert_eq!(
        PathBuf::from(value["top_entries"][0]["path"].as_str().unwrap()),
        root.join("alpha")
    );
    assert_eq!(
        PathBuf::from(value["top_entries"][1]["path"].as_str().unwrap()),
        root.join("zeta")
    );
    assert_eq!(value["top_entries"][0]["estimate_source"], "fresh-scan");
    assert_eq!(
        value["top_entries"][0]["estimate_backend"],
        "portable-recursive"
    );
    assert_eq!(value["top_entries"][0]["estimate_confidence"], "exact");
    assert_eq!(value["diagnostic_summary"]["total"], 1);
    assert_eq!(value["diagnostic_summary"]["retained"], 1);
    assert_eq!(value["diagnostic_summary"]["truncated"], 0);
    assert_eq!(
        value["diagnostic_summary"]["by_kind"][0]["kind"],
        "root-missing"
    );
    assert_eq!(value["diagnostic_summary"]["by_kind"][0]["count"], 1);
    assert_eq!(value["diagnostics"][0]["kind"], "root-missing");
}

#[test]
fn inspect_space_json_diagnostic_limit_zero_keeps_summary_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let missing = temp.path().join("missing");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "space",
            "--format",
            "json",
            "--root",
            root.to_str().unwrap(),
            "--root",
            missing.to_str().unwrap(),
            "--diagnostic-limit",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let value = &envelope["data"];
    assert_eq!(value["diagnostic_summary"]["total"], 1);
    assert_eq!(value["diagnostic_summary"]["retained"], 0);
    assert_eq!(value["diagnostic_summary"]["truncated"], 1);
    assert_eq!(
        value["diagnostic_summary"]["by_kind"][0]["kind"],
        "root-missing"
    );
    assert!(value["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn inspect_space_human_diagnostic_limit_zero_keeps_summary_count() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let missing = temp.path().join("missing");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "space",
            "--root",
            root.to_str().unwrap(),
            "--root",
            missing.to_str().unwrap(),
            "--diagnostic-limit",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Diagnostics: 1"));
    assert!(stdout.contains("Space diagnostics: 1 observation"));
    assert!(stdout.contains("  - root-missing: 1 observation"));
    assert!(stdout.contains("  - truncated: 1 observation not shown"));
    assert!(!stdout.contains("Space diagnostic samples:"));
}

#[test]
fn inspect_space_json_accepts_scan_backend_and_reports_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("target").join("app.bin"), b"abcd");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "space",
            "--format",
            "json",
            "--scan-backend",
            "windows-ntfs-mft-experimental",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let entry = &envelope["data"]["top_entries"][0];
    assert_eq!(entry["estimate_source"], "fresh-scan");
    assert!(entry["estimate_backend"].is_string());
    assert_eq!(entry["estimate_confidence"], "exact");
    if let Some(reason) = entry["estimate_fallback_reason"].as_str() {
        assert!(reason.contains("windows-ntfs-mft-experimental"));
        assert_eq!(
            entry["estimate_caveats"][0]["code"],
            "experimental-ntfs-mft-fallback"
        );
    } else {
        assert_eq!(entry["estimate_backend"], "windows-ntfs-mft-experimental");
        assert!(
            entry["estimate_backend_source"]
                .as_str()
                .is_some_and(|source| source.starts_with("windows-ntfs-mft-experimental-")),
            "entry should include the live NTFS/MFT source when no fallback occurs: {entry:#}"
        );
    }
}

#[test]
fn inspect_map_json_reports_ranked_entries_and_fallback_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("zeta").join("data.bin"), b"abc");
    write_fixture_file(root.join("alpha").join("data.bin"), b"abc");
    write_fixture_file(root.join("small.txt"), b"x");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_TEST_DISABLE_LIVE_NTFS_MFT", "1")
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "windows-ntfs-mft-experimental",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "2",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "inspect map");
    assert_eq!(envelope["payload_kind"], "inspect-map");

    let value = &envelope["data"];
    assert_eq!(value["totals"]["logical_bytes"], 7);
    assert_eq!(value["totals"]["allocated_bytes"], serde_json::Value::Null);
    assert_eq!(value["totals"]["files"], 3);
    assert_eq!(value["top_entries"].as_array().unwrap().len(), 2);
    assert_eq!(
        PathBuf::from(value["top_entries"][0]["path"].as_str().unwrap()),
        root.join("alpha")
    );
    assert_eq!(value["top_entries"][0]["depth"], 1);
    assert_eq!(value["top_entries"][0]["estimate_source"], "fresh-scan");
    assert_eq!(
        value["top_entries"][0]["estimate_backend"],
        "portable-recursive"
    );
    assert_eq!(value["top_entries"][0]["estimate_confidence"], "exact");
    assert!(
        value["top_entries"][0]["estimate_fallback_reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("windows-ntfs-mft-experimental"))
    );
    assert_eq!(value["diagnostic_summary"]["total"], 1);
    assert_eq!(value["diagnostic_summary"]["retained"], 1);
    assert_eq!(value["diagnostic_summary"]["truncated"], 0);
    assert_eq!(
        value["diagnostic_summary"]["by_kind"][0]["kind"],
        "fallback"
    );
    assert_eq!(value["diagnostic_summary"]["by_kind"][0]["count"], 1);
    assert_eq!(value["diagnostics"][0]["kind"], "fallback");
}

#[test]
fn inspect_map_json_reports_requested_groups() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("src").join("main.rs"), b"abcd");
    write_fixture_file(root.join("readme.md"), b"xy");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "0",
            "--group-by",
            "extension",
            "--group-by",
            "depth",
            "--group-by",
            "age",
            "--group-limit",
            "10",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let value = &envelope["data"];
    assert!(value["top_entries"].as_array().unwrap().is_empty());
    assert_json_group(value, "extension", ".rs", 4, 1);
    assert_json_group(value, "extension", ".md", 2, 1);
    assert_json_group(value, "depth", "depth-1", 2, 1);
    assert_json_group(value, "depth", "depth-2", 4, 1);
    assert_json_group(value, "age", "modified-7d", 6, 2);
}

#[test]
fn inspect_map_json_respects_requested_sort_fields() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("many").join("a.txt"), b"x");
    write_fixture_file(root.join("many").join("b.txt"), b"x");
    write_fixture_file(root.join("large.bin"), b"abcdefghij");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "2",
            "--sort",
            "files",
            "--group-by",
            "extension",
            "--group-limit",
            "2",
            "--group-sort",
            "files",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let value = &envelope["data"];
    assert_eq!(
        PathBuf::from(value["top_entries"][0]["path"].as_str().unwrap()),
        root.join("many")
    );
    assert_eq!(value["top_entries"][0]["files"], 2);
    assert_eq!(value["groups"][0]["key"], ".txt");
    assert_eq!(value["groups"][0]["metrics"]["files"], 2);
    assert_eq!(value["groups"][1]["key"], ".bin");
    assert_eq!(value["groups"][1]["metrics"]["logical_bytes"], 10);
}

#[test]
fn inspect_map_json_filters_ranked_entries_without_changing_totals() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("LargeCache.bin"), b"abcdef");
    write_fixture_file(root.join("small-cache.bin"), b"x");
    write_fixture_file(root.join("LargeCache").join("nested.bin"), b"xyz");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "10",
            "--min-logical-bytes",
            "4",
            "--entry-kind",
            "file",
            "--path-contains",
            "largecache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let value = &envelope["data"];
    assert_eq!(value["totals"]["logical_bytes"], 10);
    assert_eq!(value["totals"]["files"], 3);
    let entries = value["top_entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        PathBuf::from(entries[0]["path"].as_str().unwrap()),
        root.join("LargeCache.bin")
    );
    assert_eq!(entries[0]["kind"], "file");
    assert_eq!(entries[0]["logical_bytes"], 6);
}

#[test]
fn inspect_map_table_uses_ranked_entry_filters() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("LargeCache").join("nested.bin"), b"abcdef");
    write_fixture_file(root.join("other").join("nested.bin"), b"xyz");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--table",
            "csv",
            "--table-row",
            "entry",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "10",
            "--entry-kind",
            "directory",
            "--path-contains",
            "largecache",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], INSPECT_MAP_TABLE_HEADER_CSV);
    assert!(lines[1].starts_with("entry,1,"));
    assert!(lines[1].contains(&root.join("LargeCache").display().to_string()));
    assert!(!lines[1].contains("other"));
}

#[test]
fn inspect_map_json_reports_cleanup_advice_for_rule_targets() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let root = local.join("npm-cache");
    let target = root.join("_cacache");
    write_fixture_file(target.join("content.bin"), b"abcdef");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "10",
            "--entry-kind",
            "directory",
            "--path-contains",
            "_cacache",
            "--cleanup-advice",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let entries = envelope["data"]["top_entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(PathBuf::from(entries[0]["path"].as_str().unwrap()), target);
    let advice = &entries[0]["cleanup_advice"];
    assert_eq!(advice["status"], "maybe-cleanable");
    assert_eq!(advice["source"], "cleanup-rule");
    assert_eq!(advice["relation"], "exact");
    assert_eq!(advice["rule_id"], "windows.npm-cache");
    assert_eq!(advice["category"], "development");
    assert_eq!(advice["safety_level"], "moderate");
    assert_eq!(advice["required_flags"][0], "--allow-moderate");
    assert_eq!(advice["suggested_command"]["command"], "rebecca");
    assert_eq!(
        advice["suggested_command"]["args"],
        serde_json::json!(["clean", "--dry-run", "--rule", "windows.npm-cache"])
    );
}

#[test]
fn inspect_map_table_appends_cleanup_advice_columns_when_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let root = local.join("npm-cache");
    let target = root.join("_cacache");
    write_fixture_file(target.join("content.bin"), b"abcdef");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--table",
            "csv",
            "--table-row",
            "entry",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "10",
            "--entry-kind",
            "directory",
            "--path-contains",
            "_cacache",
            "--cleanup-advice",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], INSPECT_MAP_TABLE_HEADER_WITH_ADVICE_CSV);
    assert!(lines[1].contains("maybe-cleanable"));
    assert!(lines[1].contains("cleanup-rule"));
    assert!(lines[1].contains("windows.npm-cache"));
    assert!(lines[1].contains("--allow-moderate"));
    assert!(lines[1].contains("rebecca clean --dry-run --rule windows.npm-cache"));
}

#[test]
fn inspect_map_table_csv_exports_flat_rows() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace,with-comma");
    write_fixture_file(root.join("many,files").join("a.txt"), b"x");
    write_fixture_file(root.join("many,files").join("b.txt"), b"x");
    write_fixture_file(root.join("large.bin"), b"abcdefghij");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--table",
            "csv",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "1",
            "--sort",
            "files",
            "--group-by",
            "extension",
            "--group-limit",
            "1",
            "--group-sort",
            "files",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[0], INSPECT_MAP_TABLE_HEADER_CSV);
    assert!(lines[1].starts_with("total,"));
    assert!(lines[2].starts_with("root,"));
    assert!(lines[3].starts_with("entry,1,"));
    assert!(lines[4].starts_with("group,1,"));
    assert!(lines[2].contains(&format!("\"{}\"", root.display())));
    assert!(lines[3].contains(&format!("\"{}\"", root.join("many,files").display())));
    assert!(lines[4].contains(",extension,.txt,.txt,"));
    assert!(!stdout.contains('{'));
}

#[test]
fn inspect_map_table_tsv_exports_tab_separated_rows() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("src").join("main.rs"), b"abcd");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--table",
            "tsv",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "1",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], INSPECT_MAP_TABLE_HEADER_TSV);
    assert_eq!(lines[1].split('\t').next().unwrap(), "total");
    assert_eq!(lines[2].split('\t').next().unwrap(), "root");
    assert_eq!(lines[3].split('\t').next().unwrap(), "entry");
    assert!(!stdout.contains('{'));
}

#[test]
fn inspect_map_table_row_filters_selected_kinds() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("many").join("a.txt"), b"x");
    write_fixture_file(root.join("many").join("b.txt"), b"x");
    write_fixture_file(root.join("large.bin"), b"abcdefghij");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--table",
            "csv",
            "--table-row",
            "entry",
            "--table-row",
            "group",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "1",
            "--sort",
            "files",
            "--group-by",
            "extension",
            "--group-limit",
            "1",
            "--group-sort",
            "files",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], INSPECT_MAP_TABLE_HEADER_CSV);
    assert!(lines[1].starts_with("entry,1,"));
    assert!(lines[2].starts_with("group,1,"));
    let row_kinds = lines
        .iter()
        .skip(1)
        .map(|line| line.split(',').next().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(row_kinds, ["entry", "group"]);
}

#[test]
fn inspect_map_table_rejects_machine_format() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--table",
            "csv",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(common::support::stderr(&output).contains("--table"));
}

#[test]
fn inspect_map_table_row_requires_table_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--table-row",
            "entry",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(common::support::stderr(&output).contains("--table-row"));
}

fn assert_json_group(
    value: &serde_json::Value,
    kind: &str,
    key: &str,
    logical_bytes: u64,
    files: u64,
) {
    let group = value["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["kind"] == kind && group["key"] == key)
        .unwrap_or_else(|| panic!("missing group {kind}:{key}"));
    assert_eq!(group["metrics"]["logical_bytes"], logical_bytes);
    assert_eq!(group["metrics"]["files"], files);
}

const INSPECT_MAP_TABLE_HEADER_CSV: &str = "row_kind,rank,path,root,status,entry_kind,group_kind,group_key,group_label,depth,logical_bytes,allocated_bytes,unique_logical_bytes,unique_allocated_bytes,files,directories,estimate_source,estimate_backend,estimate_backend_source,estimate_confidence,estimate_fallback_reason,estimate_caveats,reason";
const INSPECT_MAP_TABLE_HEADER_WITH_ADVICE_CSV: &str = "row_kind,rank,path,root,status,entry_kind,group_kind,group_key,group_label,depth,logical_bytes,allocated_bytes,unique_logical_bytes,unique_allocated_bytes,files,directories,estimate_source,estimate_backend,estimate_backend_source,estimate_confidence,estimate_fallback_reason,estimate_caveats,reason,cleanup_status,cleanup_relation,cleanup_source,cleanup_rule_id,cleanup_category,cleanup_safety_level,cleanup_required_flags,cleanup_required_warnings,cleanup_protection_kind,cleanup_matched_path,cleanup_reason,cleanup_command";
const INSPECT_MAP_TABLE_HEADER_TSV: &str = "row_kind\trank\tpath\troot\tstatus\tentry_kind\tgroup_kind\tgroup_key\tgroup_label\tdepth\tlogical_bytes\tallocated_bytes\tunique_logical_bytes\tunique_allocated_bytes\tfiles\tdirectories\testimate_source\testimate_backend\testimate_backend_source\testimate_confidence\testimate_fallback_reason\testimate_caveats\treason";

#[cfg(windows)]
#[test]
fn inspect_map_json_windows_native_reports_native_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("alpha").join("data.bin"), b"abcd");
    write_fixture_file(root.join("beta.bin"), b"xyz");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "windows-native",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "10",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let value = &envelope["data"];
    assert_eq!(value["totals"]["logical_bytes"], 7);
    assert!(
        value["totals"]["allocated_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes >= 7)
    );
    assert_eq!(value["totals"]["files"], 2);
    assert_eq!(value["totals"]["directories"], 1);
    assert!(
        value["top_entries"][0]["allocated_bytes"]
            .as_u64()
            .is_some_and(
                |bytes| bytes >= value["top_entries"][0]["logical_bytes"].as_u64().unwrap()
            )
    );
    assert_eq!(
        value["top_entries"][0]["estimate_backend"],
        "windows-native"
    );
    assert_eq!(value["top_entries"][0]["estimate_confidence"], "exact");
    assert!(value["top_entries"][0]["estimate_fallback_reason"].is_null());
    assert!(
        value["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.is_empty())
    );
}

#[cfg(windows)]
#[test]
fn inspect_map_json_windows_native_reports_hardlink_caveat() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let original = root.join("original.bin");
    let linked = root.join("linked.bin");
    write_fixture_file(&original, b"abcd");
    fs::hard_link(&original, &linked).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "windows-native",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "10",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let value = &envelope["data"];
    assert_eq!(value["totals"]["logical_bytes"], 8);
    assert_eq!(value["totals"]["unique_logical_bytes"], 4);
    assert_eq!(value["roots"][0]["metrics"]["unique_logical_bytes"], 4);
    let allocated_bytes = value["totals"]["allocated_bytes"]
        .as_u64()
        .expect("native hardlink fixture should report path-ranked allocated bytes");
    let unique_allocated_bytes = value["totals"]["unique_allocated_bytes"]
        .as_u64()
        .expect("native hardlink fixture should report unique allocated bytes");
    assert!(
        allocated_bytes >= unique_allocated_bytes,
        "path-ranked allocation should be at least unique allocation"
    );
    assert!(
        value["roots"][0]["estimate_caveats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|caveat| caveat["code"] == "windows-native-hardlink-file")
    );
    assert!(
        value["top_entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["estimate_caveats"]
                .as_array()
                .unwrap()
                .iter()
                .any(|caveat| caveat["code"] == "windows-native-hardlink-file"))
    );
}

#[test]
fn inspect_map_json_top_zero_preserves_totals_without_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("large.bin"), b"abc");
    write_fixture_file(root.join("small.bin"), b"x");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["payload_kind"], "inspect-map");
    assert_eq!(envelope["data"]["totals"]["logical_bytes"], 4);
    assert!(
        envelope["data"]["top_entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn inspect_map_json_diagnostic_limit_zero_keeps_summary_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_TEST_DISABLE_LIVE_NTFS_MFT", "1")
        .args([
            "inspect",
            "map",
            "--format",
            "json",
            "--scan-backend",
            "windows-ntfs-mft-experimental",
            "--root",
            root.to_str().unwrap(),
            "--diagnostic-limit",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let envelope = common::support::api_envelope(&output.stdout);
    let value = &envelope["data"];
    assert_eq!(value["diagnostic_summary"]["total"], 1);
    assert_eq!(value["diagnostic_summary"]["retained"], 0);
    assert_eq!(value["diagnostic_summary"]["truncated"], 1);
    assert!(value["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn inspect_map_human_reports_diagnostic_summary_and_samples() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_TEST_DISABLE_LIVE_NTFS_MFT", "1")
        .args([
            "inspect",
            "map",
            "--scan-backend",
            "windows-ntfs-mft-experimental",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Diagnostics: 1"));
    assert!(stdout.contains("Disk map diagnostics: 1 observation"));
    assert!(stdout.contains("  - fallback: 1 observation"));
    assert!(stdout.contains("Disk map diagnostic samples: 1 observation"));
}

#[test]
fn inspect_map_human_diagnostic_limit_zero_keeps_summary_count() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_TEST_DISABLE_LIVE_NTFS_MFT", "1")
        .args([
            "inspect",
            "map",
            "--scan-backend",
            "windows-ntfs-mft-experimental",
            "--root",
            root.to_str().unwrap(),
            "--diagnostic-limit",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Diagnostics: 1"));
    assert!(stdout.contains("Disk map diagnostics: 1 observation"));
    assert!(stdout.contains("  - fallback: 1 observation"));
    assert!(stdout.contains("  - truncated: 1 observation not shown"));
    assert!(!stdout.contains("Disk map diagnostic samples:"));
}

#[test]
fn inspect_map_ndjson_uses_v1_completed_event() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .env("REBECCA_TEST_DISABLE_LIVE_NTFS_MFT", "1")
        .args([
            "inspect",
            "map",
            "--format",
            "ndjson",
            "--scan-backend",
            "windows-ntfs-mft-experimental",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "1",
            "--group-by",
            "extension",
            "--group-limit",
            "1",
            "--diagnostic-limit",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 3);

    let entry = &events[0];
    assert_eq!(entry["api_version"], "rebecca.cli.v1");
    assert_eq!(entry["event_kind"], "map-entry");
    assert_eq!(entry["command"], "inspect map");
    assert_eq!(entry["payload_kind"], "inspect-map-entry");
    assert_eq!(entry["sequence"], 0);
    assert_eq!(entry["data"]["rank"], 1);
    assert_eq!(entry["data"]["entry"]["logical_bytes"], 3);
    assert_eq!(
        entry["data"]["entry"]["estimate_backend"],
        "portable-recursive"
    );

    let group = &events[1];
    assert_eq!(group["api_version"], "rebecca.cli.v1");
    assert_eq!(group["event_kind"], "map-group");
    assert_eq!(group["command"], "inspect map");
    assert_eq!(group["payload_kind"], "inspect-map-group");
    assert_eq!(group["sequence"], 1);
    assert_eq!(group["data"]["rank"], 1);
    assert_eq!(group["data"]["group"]["kind"], "extension");
    assert_eq!(group["data"]["group"]["key"], ".bin");
    assert_eq!(group["data"]["group"]["metrics"]["logical_bytes"], 3);

    let completed = events.last().unwrap();
    assert_eq!(completed["api_version"], "rebecca.cli.v1");
    assert_eq!(completed["event_kind"], "completed");
    assert_eq!(completed["command"], "inspect map");
    assert_eq!(completed["payload_kind"], "inspect-map");
    assert_eq!(completed["sequence"], 2);
    assert_eq!(completed["data"]["totals"]["logical_bytes"], 3);
    assert_eq!(completed["data"]["diagnostic_summary"]["total"], 1);
    assert_eq!(completed["data"]["diagnostic_summary"]["retained"], 0);
    assert_eq!(completed["data"]["diagnostic_summary"]["truncated"], 1);
    assert!(
        completed["data"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn inspect_map_ndjson_advice_status_filter_implies_cleanup_advice() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("local");
    let root = local.join("npm-cache");
    let target = root.join("_cacache");
    write_fixture_file(target.join("content.bin"), b"abcdef");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "map",
            "--format",
            "ndjson",
            "--scan-backend",
            "portable-recursive",
            "--root",
            root.to_str().unwrap(),
            "--top",
            "10",
            "--entry-kind",
            "directory",
            "--path-contains",
            "_cacache",
            "--advice-status",
            "maybe-cleanable",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event_kind"], "map-entry");
    assert_eq!(
        PathBuf::from(
            events[0]["data"]["entry"]["path"]
                .as_str()
                .expect("entry path")
        ),
        target
    );
    assert_eq!(
        events[0]["data"]["entry"]["cleanup_advice"]["status"],
        "maybe-cleanable"
    );

    let completed = events.last().unwrap();
    assert_eq!(completed["event_kind"], "completed");
    assert_eq!(
        completed["data"]["top_entries"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        completed["data"]["top_entries"][0]["cleanup_advice"],
        events[0]["data"]["entry"]["cleanup_advice"]
    );
}

#[test]
fn inspect_space_ndjson_uses_v1_completed_event() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    write_fixture_file(root.join("entry.bin"), b"abc");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "space",
            "--format",
            "ndjson",
            "--root",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    let completed = events.first().unwrap();
    assert_eq!(completed["api_version"], "rebecca.cli.v1");
    assert_eq!(completed["event_kind"], "completed");
    assert_eq!(completed["command"], "inspect space");
    assert_eq!(completed["payload_kind"], "inspect-space");
    assert_eq!(completed["data"]["totals"]["estimated_bytes"], 3);
    assert_eq!(completed["data"]["diagnostic_summary"]["total"], 0);
    assert_eq!(completed["data"]["diagnostic_summary"]["retained"], 0);
    assert_eq!(completed["data"]["diagnostic_summary"]["truncated"], 0);
}

#[test]
fn inspect_artifacts_json_reports_read_only_project_artifact_insight() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let node_modules_file = workspace.join("app").join("node_modules").join("pkg.bin");
    let target_file = workspace
        .join("app")
        .join("target")
        .join("debug")
        .join("app.bin");
    write_fixture_file(&node_modules_file, b"abc");
    write_fixture_file(&target_file, b"rust");
    write_node_project(workspace.join("app"));
    write_rust_project(workspace.join("app"));

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "artifacts",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--min-age-days",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(node_modules_file.exists());
    assert!(target_file.exists());
    assert!(
        !temp
            .path()
            .join("rebecca-state")
            .join("history.jsonl")
            .exists()
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "inspect artifacts");
    assert_eq!(envelope["payload_kind"], "inspect-artifacts");

    let value = &envelope["data"];
    assert_eq!(value["summary"]["total_targets"], 2);
    assert_eq!(value["summary"]["estimated_bytes"], 7);
    assert_eq!(value["top_targets"][0]["artifact"], "target");
    assert_eq!(value["top_targets"][1]["artifact"], "node_modules");
}

#[test]
fn purge_inspect_compatibility_matches_inspect_artifacts_data() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(
        workspace.join("app").join("node_modules").join("pkg.bin"),
        b"abc",
    );
    write_node_project(workspace.join("app"));

    let inspect_output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "artifacts",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--min-age-days",
            "0",
        ])
        .output()
        .unwrap();
    let purge_output = isolated::isolated_rebecca(&temp)
        .args([
            "purge",
            "inspect",
            "--format",
            "json",
            "--no-progress",
            "--root",
            workspace.to_str().unwrap(),
            "--min-age-days",
            "0",
        ])
        .output()
        .unwrap();

    assert!(
        inspect_output.status.success(),
        "stderr: {}",
        common::support::stderr(&inspect_output)
    );
    assert!(
        purge_output.status.success(),
        "stderr: {}",
        common::support::stderr(&purge_output)
    );

    let inspect = common::support::api_envelope(&inspect_output.stdout);
    let purge = common::support::api_envelope(&purge_output.stdout);
    assert_eq!(inspect["payload_kind"], "inspect-artifacts");
    assert_eq!(purge["payload_kind"], "inspect-artifacts");
    assert_eq!(inspect["data"], purge["data"]);
}

#[test]
fn inspect_lint_json_reports_duplicates_and_empty_entries_without_writes() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let reference = temp.path().join("reference");
    let protected = temp.path().join("rebecca-state");
    let duplicate = workspace.join("copy.bin");
    let reference_duplicate = reference.join("master.bin");
    let protected_duplicate = protected.join("protected.bin");
    let empty_file = workspace.join("empty.txt");
    let large_file = workspace.join("large.bin");
    let empty_dir = workspace.join("empty").join("nested");
    write_fixture_file(&duplicate, b"same");
    write_fixture_file(&reference_duplicate, b"same");
    write_fixture_file(&protected_duplicate, b"same");
    write_fixture_file(&empty_file, b"");
    write_fixture_file(&large_file, b"abcdef");
    fs::create_dir_all(&empty_dir).unwrap();

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "lint",
            "--format",
            "json",
            "--root",
            workspace.to_str().unwrap(),
            "--root",
            protected.to_str().unwrap(),
            "--reference",
            reference.to_str().unwrap(),
            "--large-file-threshold-bytes",
            "5",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );
    assert!(duplicate.exists());
    assert!(reference_duplicate.exists());
    assert!(protected_duplicate.exists());
    assert!(
        !temp
            .path()
            .join("rebecca-state")
            .join("history.jsonl")
            .exists()
    );

    let envelope = common::support::api_envelope(&output.stdout);
    assert_eq!(envelope["command"], "inspect lint");
    assert_eq!(envelope["payload_kind"], "inspect-lint");

    let value = &envelope["data"];
    assert_eq!(value["summary"]["duplicate_groups"], 1);
    assert_eq!(value["summary"]["duplicate_files"], 3);
    assert_eq!(value["summary"]["conservative_reclaim_bytes"], 4);
    assert_eq!(value["summary"]["large_files"], 1);
    assert_eq!(value["summary"]["empty_files"], 1);
    assert!(value["summary"]["empty_directories"].as_u64().unwrap() >= 1);
    let group = &value["duplicate_groups"][0];
    assert_eq!(group["keep_candidates"], 2);
    let roles = group["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["role"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(roles, ["protected", "reference", "scanned"]);
    assert!(
        value["empty_directories"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("nested")
    );
}

#[test]
fn inspect_lint_ndjson_uses_v1_completed_event() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    write_fixture_file(workspace.join("a.bin"), b"same");
    write_fixture_file(workspace.join("b.bin"), b"same");

    let output = isolated::isolated_rebecca(&temp)
        .args([
            "inspect",
            "lint",
            "--format",
            "ndjson",
            "--root",
            workspace.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let events = stdout
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["api_version"], "rebecca.cli.v1");
    assert_eq!(events[0]["event_kind"], "completed");
    assert_eq!(events[0]["command"], "inspect lint");
    assert_eq!(events[0]["payload_kind"], "inspect-lint");
    assert_eq!(events[0]["data"]["summary"]["duplicate_groups"], 1);
}
