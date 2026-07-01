pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub fn api_envelope(stdout: &[u8]) -> serde_json::Value {
    let value: serde_json::Value = serde_json::from_slice(stdout).unwrap();
    assert_eq!(value["api_version"], "rebecca.cli.v1");
    assert_eq!(value["kind"], "success");
    value
}

#[allow(dead_code)]
pub fn api_data(stdout: &[u8]) -> serde_json::Value {
    api_envelope(stdout)["data"].clone()
}
