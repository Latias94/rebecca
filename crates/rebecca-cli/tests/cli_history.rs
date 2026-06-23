mod isolated;
mod support;

#[test]
fn history_json_is_empty_when_no_history_file_exists() {
    let temp = tempfile::tempdir().unwrap();
    let output = isolated::isolated_rebecca(&temp)
        .args(["history", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        support::stderr(&output)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(value.as_array().unwrap().len(), 0);
}
