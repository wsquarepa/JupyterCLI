use crate::api::server::ServerClient;
use crate::api::ws::TermSocket;
use crate::attach::{self, AttachOutcome};
use crate::shellops;

use super::addr::parse_shell_ref;
use super::{CliError, Ctx, ShellCmd};

pub async fn server_client_for(
    ctx: &Ctx,
    server: Option<&str>,
) -> Result<(ServerClient, String), CliError> {
    let user = ctx.client.whoami().await?;
    let key = server.unwrap_or_default().to_string();
    let display = if key.is_empty() {
        "default".to_string()
    } else {
        key.clone()
    };
    let found = user.servers.get(&key).ok_or_else(|| {
        CliError::Usage(format!(
            "server '{display}' is not running; start it with: jhc start"
        ))
    })?;
    if !found.ready {
        return Err(CliError::Usage(format!(
            "server '{display}' is still {}; wait for it to become ready",
            found.pending.as_deref().unwrap_or("pending")
        )));
    }
    let url_path = found
        .url
        .as_deref()
        .ok_or_else(|| CliError::Usage(format!("server '{display}' reports no URL")))?;
    Ok((ServerClient::from_hub(&ctx.client, url_path)?, display))
}

async fn connect(client: &ServerClient, ctx: &Ctx, shell: &str) -> Result<TermSocket, CliError> {
    let url = client.ws_terminal_url(shell)?;
    Ok(TermSocket::connect(&url, &ctx.hub.effective_token()).await?)
}

pub async fn run(ctx: &Ctx, cmd: ShellCmd) -> Result<(), CliError> {
    match cmd {
        ShellCmd::New { server } => {
            let (client, display) = server_client_for(ctx, server.as_deref()).await?;
            let term = client.create_terminal().await?;
            println!("created shell {} on server {display}", term.name);
            Ok(())
        }
        ShellCmd::List { server } => {
            let (client, display) = server_client_for(ctx, server.as_deref()).await?;
            let terminals = client.terminals().await?;
            if terminals.is_empty() {
                println!("no shells on server {display}");
                return Ok(());
            }
            println!("{:<8} LAST ACTIVITY", "SHELL");
            for term in terminals {
                println!(
                    "{:<8} {}",
                    term.name,
                    term.last_activity.as_deref().unwrap_or("-")
                );
            }
            Ok(())
        }
        ShellCmd::Send { shell, text } => {
            let (server, name) = parse_shell_ref(&shell);
            let (client, _) = server_client_for(ctx, server.as_deref()).await?;
            let sock = connect(&client, ctx, &name).await?;
            shellops::send(sock, &text.join(" ")).await?;
            Ok(())
        }
        ShellCmd::Peek { shell, follow, raw } => {
            let (server, name) = parse_shell_ref(&shell);
            let (client, _) = server_client_for(ctx, server.as_deref()).await?;
            let sock = connect(&client, ctx, &name).await?;
            let mut stdout = std::io::stdout();
            if follow {
                tokio::select! {
                    result = shellops::peek(sock, raw, true, &mut stdout) => result?,
                    _ = tokio::signal::ctrl_c() => {}
                }
            } else {
                shellops::peek(sock, raw, false, &mut stdout).await?;
            }
            Ok(())
        }
        ShellCmd::Kill { shell } => {
            let (server, name) = parse_shell_ref(&shell);
            let (client, display) = server_client_for(ctx, server.as_deref()).await?;
            client.delete_terminal(&name).await?;
            println!("killed shell {name} on server {display}");
            Ok(())
        }
        ShellCmd::Attach { shell } => {
            use std::io::IsTerminal as _;
            if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
                return Err(CliError::Usage(
                    "no TTY: use jhc exec for scripted commands".to_string(),
                ));
            }
            let (server, name) = parse_shell_ref(&shell);
            let (client, _) = server_client_for(ctx, server.as_deref()).await?;
            let sock = connect(&client, ctx, &name).await?;
            match attach::attach(sock).await? {
                AttachOutcome::Detached => println!("\ndetached; the shell keeps running"),
                AttachOutcome::RemoteClosed => println!("\nthe shell exited"),
            }
            Ok(())
        }
    }
}

pub async fn exec_cmd(
    ctx: &Ctx,
    server: Option<&str>,
    shell: Option<&str>,
    command: &str,
) -> Result<i32, CliError> {
    use std::io::IsTerminal as _;

    let (reuse_server, reuse_shell) = match shell {
        Some(reference) => {
            let (ref_server, ref_shell) = parse_shell_ref(reference);
            if let (Some(a), Some(b)) = (server, ref_server.as_deref())
                && a != b
            {
                return Err(CliError::Usage(format!(
                    "conflicting servers: positional '{a}' vs --shell '{b}'"
                )));
            }
            (
                ref_server.or_else(|| server.map(String::from)),
                Some(ref_shell),
            )
        }
        None => (server.map(String::from), None),
    };

    let (client, _) = server_client_for(ctx, reuse_server.as_deref()).await?;
    let (shell_name, ephemeral) = match reuse_shell {
        Some(name) => (name, false),
        None => (client.create_terminal().await?.name, true),
    };

    let stdin_pipe = if std::io::stdin().is_terminal() {
        None
    } else {
        Some(tokio::io::stdin())
    };

    let url = client.ws_terminal_url(&shell_name)?;
    let sock = TermSocket::connect(&url, &ctx.hub.effective_token()).await?;
    let mut stdout = std::io::stdout();
    let result = shellops::exec(sock, command, stdin_pipe, &mut stdout).await;

    if ephemeral {
        // The remote `exit` self-destructs the terminal; DELETE is the belt for
        // error paths and for terminado versions that keep exited terminals listed.
        if let Err(cleanup) = client.delete_terminal(&shell_name).await {
            eprintln!("warning: could not clean up shell {shell_name}: {cleanup}");
        }
    }
    Ok(result?.exit_code)
}
