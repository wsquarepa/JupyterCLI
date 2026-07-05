use std::path::PathBuf;

pub fn write_config(dir: &std::path::Path, hub_url: &str) -> PathBuf {
    write_config_with(dir, hub_url, "")
}

/// `extra_hub_lines` land inside the `[hubs.test]` table, before the presets
/// table (TOML keys after a table header would belong to that table).
pub fn write_config_with(dir: &std::path::Path, hub_url: &str, extra_hub_lines: &str) -> PathBuf {
    let text = format!(
        "default_hub = \"test\"\n\n[hubs.test]\nurl = \"{hub_url}\"\ntoken = \"tok\"\n{extra_hub_lines}\n[hubs.test.presets.gpu]\nprofile = \"environments\"\nresource = \"2_a100\"\n"
    );
    let path = dir.join("config.toml");
    std::fs::write(&path, text).unwrap();
    path
}
