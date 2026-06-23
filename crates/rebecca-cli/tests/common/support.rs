use std::process::Command;

pub fn rebecca() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rebecca"))
}

pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
