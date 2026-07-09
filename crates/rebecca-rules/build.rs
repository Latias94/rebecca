use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn main() -> io::Result<()> {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set by Cargo"),
    );
    let rules_dir = manifest_dir.join("rules").join("cleanup");
    let mut rule_paths = cleanup_rule_paths(&rules_dir)?;

    println!("cargo:rerun-if-changed=rules/cleanup");
    for path in &rule_paths {
        println!("cargo:rerun-if-changed={path}");
    }

    rule_paths.sort();

    let mut generated = String::from("pub(crate) const BUILTIN_RULE_FILES: &[(&str, &str)] = &[\n");
    for path in rule_paths {
        generated.push_str("    (\n");
        generated.push_str(&format!("        {path:?},\n"));
        generated.push_str(&format!(
            "        include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{path}\")),\n"
        ));
        generated.push_str("    ),\n");
    }
    generated.push_str("];\n");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR should be set by Cargo"));
    fs::write(out_dir.join("builtin_rule_files.rs"), generated)
}

fn cleanup_rule_paths(rules_dir: &Path) -> io::Result<Vec<String>> {
    let mut paths = Vec::new();

    for entry in fs::read_dir(rules_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            let file_name = path
                .file_name()
                .expect("rule path should have a file name")
                .to_string_lossy();
            paths.push(format!("rules/cleanup/{file_name}"));
        }
    }

    if paths.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rules/cleanup must contain at least one TOML rule file",
        ));
    }

    Ok(paths)
}
