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
    _ctx: &Ctx,
    _server: Option<&str>,
    _preset: Option<&str>,
    _options: &[String],
    _no_wait: bool,
) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}

pub async fn stop(_ctx: &Ctx, _server: Option<&str>) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}
