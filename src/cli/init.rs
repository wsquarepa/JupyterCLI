use crate::api::HubClient;
use crate::config::{self, Config, HubConfig};

use super::CliError;

pub const NO_CONFIG_GUIDANCE: &str = "JupyterCLI is not configured yet.

To get started you need two things:
  1. Your hub's base URL, e.g. https://jupyter.example.edu
  2. An API token: create one in the browser at <hub url>/hub/token

Then run:
  jhc init
or non-interactively:
  jhc init --url https://jupyter.example.edu --token YOUR_TOKEN

The configuration is written to ~/.config/jhc/config.toml with owner-only permissions.";

pub async fn run(url: Option<String>, token: Option<String>, name: String) -> Result<(), CliError> {
    let url = match url {
        Some(u) => u,
        None => prompt("Hub base URL (e.g. https://jupyter.example.edu): ")?,
    };
    let token = match token {
        Some(t) => t,
        None => {
            println!(
                "Create a token in the browser at {}/hub/token",
                url.trim_end_matches('/')
            );
            rpassword::prompt_password("API token (input hidden): ").map_err(CliError::Io)?
        }
    };

    let client = HubClient::new(&url, &token)?;
    let user = client.whoami().await?;
    println!("Connected to {url} as {}", user.name);

    let mut cfg = match config::load() {
        Ok(existing) => existing,
        Err(config::ConfigError::NotFound(_)) => Config {
            default_hub: name.clone(),
            hubs: Default::default(),
        },
        Err(e) => return Err(e.into()),
    };
    // Re-running init to refresh a token must not discard presets imported for an
    // existing hub of the same name.
    let presets = cfg
        .hubs
        .get(&name)
        .map(|existing| existing.presets.clone())
        .unwrap_or_default();
    cfg.hubs.insert(
        name.clone(),
        HubConfig {
            url: url.clone(),
            token,
            terminal_limit: None,
            presets,
        },
    );
    if !cfg.hubs.contains_key(&cfg.default_hub) {
        cfg.default_hub = name.clone();
    }
    let path = config::save(&cfg)?;
    println!("Saved hub '{name}' to {}", path.display());

    let running: Vec<_> = user.servers.values().filter(|s| s.ready).collect();
    match running.first() {
        Some(server) if !server.user_options.is_empty() => {
            let options = serde_json::to_string(&server.user_options).unwrap_or_default();
            println!(
                "Found a running server with options {options}.\nSave them as a preset with: jhc preset import {}",
                if server.name.is_empty() {
                    ""
                } else {
                    &server.name
                }
            );
        }
        _ => {
            println!(
                "Note: JupyterCLI cannot list your hub's environment and resource options\nbecause JupyterHub does not expose them over its API. Start a server once from\n{}/hub/spawn in the browser, then run: jhc preset import",
                url.trim_end_matches('/')
            );
        }
    }
    Ok(())
}

fn prompt(message: &str) -> Result<String, CliError> {
    use std::io::Write as _;
    print!("{message}");
    std::io::stdout().flush().map_err(CliError::Io)?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(CliError::Io)?;
    Ok(line.trim().to_string())
}
