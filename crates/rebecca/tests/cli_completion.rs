mod common;

const SUPPORTED_SHELLS: &[&str] = &["bash", "elvish", "fish", "powershell", "zsh"];
const TOP_LEVEL_COMMANDS: &[&str] = &["scan", "clean", "tui", "purge", "trash", "completion"];

#[test]
fn completion_help_lists_supported_shells() {
    let output = common::command::rebecca()
        .args(["completion", "--help"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        common::support::stderr(&output)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    for shell in SUPPORTED_SHELLS {
        assert!(stdout.contains(shell), "help should list {shell}: {stdout}");
    }
}

#[test]
fn completion_generation_covers_all_supported_shells_and_current_subcommands() {
    for shell in SUPPORTED_SHELLS {
        let stdout = generate_completion(shell);
        assert!(
            !stdout.trim().is_empty(),
            "{shell} completion should not be empty"
        );
        assert!(
            stdout.contains("rebecca"),
            "{shell} completion should reference the binary name"
        );
        for command in TOP_LEVEL_COMMANDS {
            assert!(
                stdout.contains(command),
                "{shell} completion should include top-level command {command}"
            );
        }
    }
}

#[test]
fn completion_generation_includes_path_completion_hints() {
    let zsh = generate_completion("zsh");

    assert!(
        zsh.contains("--root") && zsh.contains("_files"),
        "zsh completion should use filesystem completion for path roots: {zsh}"
    );
    assert!(
        zsh.contains("--file") && zsh.contains("_files"),
        "zsh completion should use filesystem completion for file arguments: {zsh}"
    );
}

#[test]
fn completion_invalid_shell_is_rejected() {
    let output = common::command::rebecca()
        .args(["completion", "invalid"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = common::support::stderr(&output);
    assert!(stderr.contains("invalid value"));
    assert!(stderr.contains("powershell"));
}

fn generate_completion(shell: &str) -> String {
    let output = common::command::rebecca()
        .args(["completion", shell])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{shell} completion failed, stderr: {}",
        common::support::stderr(&output)
    );

    String::from_utf8(output.stdout).unwrap()
}
