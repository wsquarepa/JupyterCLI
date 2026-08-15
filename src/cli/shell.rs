use crate::api::server::ServerClient;
use crate::api::types::Server;
use crate::api::ws::TermSocket;
use crate::attach::{self, AttachOutcome};
use crate::shellops;
use tokio::signal::unix::{Signal, SignalKind};

use super::addr::parse_shell_ref;
use super::{CliError, Ctx, ShellCmd};

pub async fn server_entry_for(
    ctx: &Ctx,
    server: Option<&str>,
) -> Result<(Server, String), CliError> {
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
    Ok((found.clone(), display))
}

pub fn client_for_entry(
    ctx: &Ctx,
    entry: &Server,
    display: &str,
) -> Result<ServerClient, CliError> {
    let url_path = entry
        .url
        .as_deref()
        .ok_or_else(|| CliError::Usage(format!("server '{display}' reports no URL")))?;
    Ok(ServerClient::from_hub(&ctx.client, url_path)?)
}

pub async fn server_client_for(
    ctx: &Ctx,
    server: Option<&str>,
) -> Result<(ServerClient, String), CliError> {
    let (entry, display) = server_entry_for(ctx, server).await?;
    Ok((client_for_entry(ctx, &entry, &display)?, display))
}

async fn connect(client: &ServerClient, ctx: &Ctx, shell: &str) -> Result<TermSocket, CliError> {
    let url = client.ws_terminal_url(shell)?;
    Ok(TermSocket::connect(&url, &ctx.hub.effective_token()).await?)
}

// Registering the listener installs tokio's process-wide SIGINT handler, so from this
// point on a Ctrl-C only has an effect where something awaits `recv()`; callers create
// it right before the first await they want interruptible and keep it alive until the
// last one. Multiple listeners can coexist; each is notified independently.
pub fn interrupt_listener() -> Result<Signal, CliError> {
    Ok(tokio::signal::unix::signal(SignalKind::interrupt())?)
}

// Creates a throwaway terminal and connects to it. A Ctrl-C during either step returns
// `CliError::Interrupted` with the terminal already deleted; without this the process
// would die between create and connect (before exec's own handler exists) and leak
// the terminal against the server's terminal_limit.
pub async fn open_ephemeral(
    ctx: &Ctx,
    client: &ServerClient,
    interrupt: &mut Signal,
) -> Result<(String, TermSocket), CliError> {
    let name = tokio::select! {
        biased;
        _ = interrupt.recv() => return Err(CliError::Interrupted),
        created = client.create_terminal() => created?.name,
    };
    let connected = tokio::select! {
        biased;
        _ = interrupt.recv() => Err(CliError::Interrupted),
        sock = connect(client, ctx, &name) => sock,
    };
    match connected {
        Ok(sock) => Ok((name, sock)),
        Err(e) => {
            close_ephemeral(client, &name).await;
            Err(e)
        }
    }
}

// The remote `exit` self-destructs the terminal; DELETE is the belt for error paths,
// interrupted execs, and terminado versions that keep exited terminals listed.
pub async fn close_ephemeral(client: &ServerClient, name: &str) {
    if let Err(cleanup) = client.delete_terminal(name).await {
        eprintln!("warning: could not clean up shell {name}: {cleanup}");
    }
}

pub async fn run(ctx: &Ctx, cmd: ShellCmd) -> Result<(), CliError> {
    match cmd {
        ShellCmd::New { server } => {
            let (client, display) = server_client_for(ctx, server.as_deref()).await?;
            let limit = ctx.hub.effective_terminal_limit();
            let count = client.terminals().await?.len();
            if count >= limit {
                return Err(CliError::Usage(format!(
                    "terminal limit reached ({count} of {limit}); raise terminal_limit in the config to allow more"
                )));
            }
            let term = client.create_terminal().await?;
            println!("created shell {} on server {display}", term.name);
            Ok(())
        }
        ShellCmd::List { server, json } => {
            let (client, display) = server_client_for(ctx, server.as_deref()).await?;
            let terminals = client.terminals().await?;
            if json {
                let payload = serde_json::json!({"terminals": terminals});
                println!("{payload}");
                return Ok(());
            }
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
        ShellCmd::Peek {
            shell,
            follow,
            raw,
            max_wait,
            max_bytes,
        } => {
            let (server, name) = parse_shell_ref(&shell);
            let (client, _) = server_client_for(ctx, server.as_deref()).await?;
            let sock = connect(&client, ctx, &name).await?;
            let mut stdout = std::io::stdout();
            if follow {
                let wait = max_wait.map(std::time::Duration::from_secs);
                tokio::select! {
                    result = shellops::peek(sock, raw, true, wait, max_bytes, &mut stdout) => result?,
                    _ = tokio::signal::ctrl_c() => {}
                }
            } else {
                shellops::peek(sock, raw, false, None, None, &mut stdout).await?;
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
    let (shell_name, ephemeral, sock) = match reuse_shell {
        Some(name) => {
            let sock = connect(&client, ctx, &name).await?;
            (name, false, sock)
        }
        None => {
            let mut interrupt = interrupt_listener()?;
            let (name, sock) = open_ephemeral(ctx, &client, &mut interrupt).await?;
            (name, true, sock)
        }
    };

    let stdin_pipe = if std::io::stdin().is_terminal() {
        None
    } else {
        Some(tokio::io::stdin())
    };

    let mut stdout = std::io::stdout();
    let result = shellops::exec(sock, command, stdin_pipe, ephemeral, &mut stdout).await;

    if ephemeral {
        close_ephemeral(&client, &shell_name).await;
    }
    Ok(result?.exit_code)
}
