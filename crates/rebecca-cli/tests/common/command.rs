use std::process::Command;

pub fn rebecca() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rebecca"))
}
