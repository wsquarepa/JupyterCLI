use std::path::PathBuf;
use std::process::Command;

pub fn client_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jhc"))
}

#[expect(
    dead_code,
    reason = "reserved for the cp test that a later task in this plan adds"
)]
pub fn write_config(dir: &std::path::Path, hub_url: &str) -> PathBuf {
    let text = format!(
        "default_hub = \"test\"\n\n[hubs.test]\nurl = \"{hub_url}\"\ntoken = \"tok\"\n\n[hubs.test.presets.gpu]\nprofile = \"environments\"\nresource = \"2_a100\"\n"
    );
    let path = dir.join("config.toml");
    std::fs::write(&path, text).unwrap();
    path
}
