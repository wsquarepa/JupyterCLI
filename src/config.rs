use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use crate::api::types::JsonMap;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no configuration found at {0}")]
    NotFound(PathBuf),
    #[error("cannot determine the config directory: no home directory")]
    NoConfigDir,
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid config {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("config serialization failed: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("unknown hub '{name}': configured hubs are {available}")]
    UnknownHub { name: String, available: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub default_hub: String,
    #[serde(default)]
    pub hubs: BTreeMap<String, HubConfig>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HubConfig {
    pub url: String,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_limit: Option<u32>,
    #[serde(default)]
    pub presets: BTreeMap<String, JsonMap>,
}

impl Config {
    pub fn resolve_hub(&self, name: Option<&str>) -> Result<(&str, &HubConfig), ConfigError> {
        let wanted = name.unwrap_or(&self.default_hub);
        match self.hubs.get_key_value(wanted) {
            Some((k, v)) => Ok((k.as_str(), v)),
            None => Err(ConfigError::UnknownHub {
                name: wanted.to_string(),
                available: self.hubs.keys().cloned().collect::<Vec<_>>().join(", "),
            }),
        }
    }
}

impl HubConfig {
    pub fn effective_token(&self) -> String {
        std::env::var("JUPYTERHUB_API_TOKEN").unwrap_or_else(|_| self.token.clone())
    }

    /// Terminals allowed per server: the config's `terminal_limit` when set
    /// (it may raise past or lower below the default), else 999.
    pub fn effective_terminal_limit(&self) -> usize {
        self.terminal_limit
            .map(|v| v as usize)
            .unwrap_or(crate::shellops::TERMINAL_LIMIT)
    }
}

pub fn dir() -> Result<PathBuf, ConfigError> {
    if let Ok(over) = std::env::var("JHC_CONFIG_DIR") {
        return Ok(PathBuf::from(over));
    }
    dirs::config_dir()
        .map(|d| d.join("jhc"))
        .ok_or(ConfigError::NoConfigDir)
}

pub fn path() -> Result<PathBuf, ConfigError> {
    Ok(dir()?.join("config.toml"))
}

pub fn load() -> Result<Config, ConfigError> {
    load_from(&dir()?)
}

pub fn save(cfg: &Config) -> Result<PathBuf, ConfigError> {
    save_to(cfg, &dir()?)
}

fn load_from(dir: &Path) -> Result<Config, ConfigError> {
    let path = dir.join("config.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConfigError::NotFound(path));
        }
        Err(e) => return Err(ConfigError::Read { path, source: e }),
    };
    let cfg = toml::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.clone(),
        source: e,
    })?;
    tracing::debug!(target: "jhc::config", path = %path.display(), "config loaded");
    Ok(cfg)
}

fn save_to(cfg: &Config, dir: &Path) -> Result<PathBuf, ConfigError> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join("config.toml");
    std::fs::create_dir_all(dir).map_err(|e| ConfigError::Write {
        path: path.clone(),
        source: e,
    })?;
    let text = toml::to_string_pretty(cfg)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| ConfigError::Write {
            path: path.clone(),
            source: e,
        })?;
    file.write_all(text.as_bytes())
        .map_err(|e| ConfigError::Write {
            path: path.clone(),
            source: e,
        })?;
    tracing::debug!(target: "jhc::config", path = %path.display(), "config saved");
    Ok(path)
}

pub fn add_preset(hub_name: &str, name: &str, options: JsonMap) -> Result<(), ConfigError> {
    add_preset_in(&dir()?, hub_name, name, options)
}

fn add_preset_in(
    dir: &Path,
    hub_name: &str,
    name: &str,
    options: JsonMap,
) -> Result<(), ConfigError> {
    let mut cfg = load_from(dir)?;
    if !cfg.hubs.contains_key(hub_name) {
        return Err(ConfigError::UnknownHub {
            name: hub_name.to_string(),
            available: cfg.hubs.keys().cloned().collect::<Vec<_>>().join(", "),
        });
    }
    let hub = cfg.hubs.get_mut(hub_name).expect("existence checked above");
    hub.presets.insert(name.to_string(), options);
    save_to(&cfg, dir)?;
    Ok(())
}

pub fn permissions_are_loose(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.permissions().mode() & 0o077 != 0,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> &'static str {
        r#"
default_hub = "icrn"

[hubs.icrn]
url = "https://jupyter.example.edu"
token = "tok123"

[hubs.icrn.presets.a100]
profile = "environments"
image = "vscode"
gpus = 2
debug = true
"#
    }

    #[test]
    fn parses_presets_with_typed_values() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        let preset = &cfg.hubs["icrn"].presets["a100"];
        assert_eq!(preset["profile"], serde_json::json!("environments"));
        assert_eq!(preset["gpus"], serde_json::json!(2));
        assert_eq!(preset["debug"], serde_json::json!(true));
    }

    #[test]
    fn rejects_unknown_keys() {
        let bad = "default_hub = \"x\"\nsurprise = 1\n";
        assert!(toml::from_str::<Config>(bad).is_err());
    }

    #[test]
    fn resolve_hub_default_and_override() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        assert_eq!(cfg.resolve_hub(None).unwrap().0, "icrn");
        assert_eq!(cfg.resolve_hub(Some("icrn")).unwrap().0, "icrn");
        let err = cfg.resolve_hub(Some("nope")).unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn save_writes_0600_and_roundtrips() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cfg: Config = toml::from_str(sample()).unwrap();
        let path = save_to(&cfg, dir.path()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let loaded = load_from(dir.path()).unwrap();
        assert_eq!(loaded.default_hub, "icrn");
        assert!(!permissions_are_loose(&path));
    }

    #[test]
    fn load_missing_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            load_from(dir.path()),
            Err(ConfigError::NotFound(_))
        ));
    }

    #[test]
    fn terminal_limit_parses_and_defaults() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        assert_eq!(cfg.hubs["icrn"].effective_terminal_limit(), 999);

        let with_limit = r#"
default_hub = "icrn"

[hubs.icrn]
url = "https://jupyter.example.edu"
token = "tok123"
terminal_limit = 2000
"#;
        let cfg: Config = toml::from_str(with_limit).unwrap();
        assert_eq!(cfg.hubs["icrn"].effective_terminal_limit(), 2000);
    }

    #[test]
    fn add_preset_inserts_and_preserves_existing() {
        let dir = tempfile::tempdir().unwrap();
        let cfg: Config = toml::from_str(sample()).unwrap();
        save_to(&cfg, dir.path()).unwrap();

        let options: JsonMap = serde_json::from_str(r#"{"resource":"3_h200"}"#).unwrap();
        add_preset_in(dir.path(), "icrn", "h200", options).unwrap();

        let loaded = load_from(dir.path()).unwrap();
        assert_eq!(
            loaded.hubs["icrn"].presets["h200"]["resource"],
            serde_json::json!("3_h200")
        );
        assert!(
            loaded.hubs["icrn"].presets.contains_key("a100"),
            "existing preset must survive"
        );
    }

    #[test]
    fn add_preset_unknown_hub_errors() {
        let dir = tempfile::tempdir().unwrap();
        let cfg: Config = toml::from_str(sample()).unwrap();
        save_to(&cfg, dir.path()).unwrap();
        let err = add_preset_in(dir.path(), "nope", "x", JsonMap::new()).unwrap_err();
        assert!(matches!(err, ConfigError::UnknownHub { .. }));
    }
}
