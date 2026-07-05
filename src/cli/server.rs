use super::{CliError, Ctx};

pub async fn status(ctx: &Ctx) -> Result<(), CliError> {
    let user = ctx.client.whoami().await?;
    println!("hub {} ({}) user {}", ctx.hub_name, ctx.hub.url, user.name);
    if user.servers.is_empty() {
        println!("no servers running");
        return Ok(());
    }
    println!("{:<12} {:<10} OPTIONS", "SERVER", "STATE");
    for (name, server) in &user.servers {
        let display = if name.is_empty() { "default" } else { name };
        let state = if server.ready {
            "ready"
        } else if server.pending.is_some() {
            server.pending.as_deref().unwrap_or("pending")
        } else {
            "stopped"
        };
        let options = if server.user_options.is_empty() {
            "-".to_string()
        } else {
            server
                .user_options
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        println!("{display:<12} {state:<10} {options}");
    }
    Ok(())
}

pub async fn start(
    ctx: &Ctx,
    server: Option<&str>,
    preset: Option<&str>,
    options: &[String],
    no_wait: bool,
) -> Result<(), CliError> {
    use crate::config::JsonMap;

    let spawn_options: JsonMap = match (preset, options.is_empty()) {
        (Some(_), false) => {
            return Err(CliError::Usage(
                "pass either --preset or -o options, not both".to_string(),
            ));
        }
        (Some(name), true) => match ctx.hub.presets.get(name) {
            Some(preset) => preset.clone(),
            None => {
                return Err(CliError::Usage(format!(
                    "unknown preset '{name}': configured presets are {}",
                    ctx.hub
                        .presets
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        },
        (None, false) => super::options_from_flags(options)?,
        (None, true) => JsonMap::new(),
    };

    let user = ctx.client.whoami().await?;
    ctx.client.spawn(&user.name, server, &spawn_options).await?;
    println!("spawn requested for {}", server.unwrap_or("default server"));
    if no_wait {
        return Ok(());
    }
    ctx.client
        .wait_ready(&user.name, server, |event| {
            if let Some(message) = &event.message {
                let pct = event
                    .progress
                    .map(|p| format!("[{p:>3}%] "))
                    .unwrap_or_default();
                println!("{pct}{message}");
            }
        })
        .await?;
    println!("server ready");
    Ok(())
}

pub async fn stop(ctx: &Ctx, server: Option<&str>) -> Result<(), CliError> {
    let user = ctx.client.whoami().await?;
    ctx.client.stop(&user.name, server).await?;
    println!("stop requested for {}", server.unwrap_or("default server"));
    Ok(())
}
