use std::path::PathBuf;

pub fn write_config(dir: &std::path::Path, hub_url: &str) -> PathBuf {
    let text = format!(
        "default_hub = \"test\"\n\n[hubs.test]\nurl = \"{hub_url}\"\ntoken = \"tok\"\n\n[hubs.test.presets.gpu]\nprofile = \"environments\"\nresource = \"2_a100\"\n"
    );
    let path = dir.join("config.toml");
    std::fs::write(&path, text).unwrap();
    path
}
