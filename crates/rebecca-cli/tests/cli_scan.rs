use std::process::Command;

#[test]
fn scan_json_lists_builtin_rules() {
    let output = rebecca().args(["scan", "--json"]).output().unwrap();

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rules = value.as_array().expect("scan output should be an array");

    assert!(rules.iter().any(|rule| rule["id"] == "windows.user-temp"));
}

fn rebecca() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rebecca"))
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
