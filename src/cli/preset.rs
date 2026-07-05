use super::{CliError, Ctx, PresetCmd};
use crate::config;

pub async fn run(ctx: &Ctx, cmd: PresetCmd) -> Result<(), CliError> {
    match cmd {
        PresetCmd::List => {
            if ctx.hub.presets.is_empty() {
                println!(
                    "no presets configured for hub {}; capture one from a running server with: jhc preset import",
                    ctx.hub_name
                );
                return Ok(());
            }
            for (name, options) in &ctx.hub.presets {
                let rendered = options
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                println!("{name}: {rendered}");
            }
            Ok(())
        }
        PresetCmd::Import { server, name } => {
            let user = ctx.client.whoami().await?;
            let key = server.clone().unwrap_or_default();
            let found = user.servers.get(&key).ok_or_else(|| {
                CliError::Usage(format!(
                    "server '{}' is not running; start it from the browser first, then import",
                    if key.is_empty() { "default" } else { &key }
                ))
            })?;
            if found.user_options.is_empty() {
                return Err(CliError::Usage(
                    "the running server reports no user options; nothing to import".to_string(),
                ));
            }
            let mut cfg = config::load()?;
            let hub = cfg.hubs.get_mut(&ctx.hub_name).ok_or_else(|| {
                CliError::Usage(format!("hub '{}' vanished from the config", ctx.hub_name))
            })?;
            hub.presets.insert(name.clone(), found.user_options.clone());
            config::save(&cfg)?;
            println!(
                "saved preset '{name}' for hub {} from server '{}'",
                ctx.hub_name,
                if key.is_empty() { "default" } else { &key }
            );
            Ok(())
        }
    }
}
