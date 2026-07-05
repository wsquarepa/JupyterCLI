use std::process::Command;

pub fn client_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jhc"))
}
