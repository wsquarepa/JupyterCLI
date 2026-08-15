use std::process::ExitCode;

use crate::api::server::ServerClient;
use crate::api::ws::TermSocket;
use crate::jobops::{self, JobMeta};
use crate::shellops;

use super::addr::parse_shell_ref;
use super::shell::server_entry_for;
use super::{CliError, Ctx, JobCmd};

struct JobTarget {
    client: ServerClient,
    display: String,
}

async fn resolve_target(ctx: &Ctx, server: Option<&str>) -> Result<JobTarget, CliError> {
    let (entry, display) = server_entry_for(ctx, server).await?;
    let url_path = entry
        .url
        .as_deref()
        .ok_or_else(|| CliError::Usage(format!("server '{display}' reports no URL")))?;
    let client = ServerClient::from_hub(&ctx.client, url_path)?;
    Ok(JobTarget { client, display })
}

struct ScriptOutcome {
    output: String,
    exit_code: i32,
}

async fn run_script_on(
    ctx: &Ctx,
    target: &JobTarget,
    script: &str,
) -> Result<ScriptOutcome, CliError> {
    let name = target.client.create_terminal().await?.name;
    let url = target.client.ws_terminal_url(&name)?;
    let sock = TermSocket::connect(&url, &ctx.hub.effective_token()).await?;
    let mut buf: Vec<u8> = Vec::new();
    let result = shellops::exec(sock, script, None, true, &mut buf).await;
    // Same belt as exec_cmd: the remote `exit` self-destructs the terminal, DELETE
    // covers error paths and terminado versions that keep exited terminals listed.
    if let Err(cleanup) = target.client.delete_terminal(&name).await {
        eprintln!("warning: could not clean up shell {name}: {cleanup}");
    }
    let outcome = result?;
    Ok(ScriptOutcome {
        output: String::from_utf8_lossy(&buf).into_owned(),
        exit_code: outcome.exit_code,
    })
}

fn job_ref(raw: &str) -> Result<(Option<String>, String), CliError> {
    let (server, id) = parse_shell_ref(raw);
    jobops::validate_job_id(&id).map_err(CliError::Usage)?;
    Ok((server, id))
}

pub async fn run(ctx: &Ctx, cmd: JobCmd) -> Result<ExitCode, CliError> {
    match cmd {
        JobCmd::Start {
            server,
            name,
            json,
            command,
        } => {
            start(ctx, server.as_deref(), name, json, &command).await?;
            Ok(ExitCode::SUCCESS)
        }
        JobCmd::List { .. } => Err(CliError::Usage(
            "job list is not implemented yet".to_string(),
        )),
        JobCmd::Status { job, .. } => {
            job_ref(&job)?;
            Err(CliError::Usage(
                "job status is not implemented yet".to_string(),
            ))
        }
        JobCmd::Tail { job, .. } => {
            job_ref(&job)?;
            Err(CliError::Usage(
                "job tail is not implemented yet".to_string(),
            ))
        }
        JobCmd::Wait { job, .. } => {
            job_ref(&job)?;
            Err(CliError::Usage(
                "job wait is not implemented yet".to_string(),
            ))
        }
        JobCmd::Kill { job, .. } => {
            job_ref(&job)?;
            Err(CliError::Usage(
                "job kill is not implemented yet".to_string(),
            ))
        }
        JobCmd::Rm { job, .. } => {
            job_ref(&job)?;
            Err(CliError::Usage("job rm is not implemented yet".to_string()))
        }
        JobCmd::Clean { .. } => Err(CliError::Usage(
            "job clean is not implemented yet".to_string(),
        )),
    }
}

async fn start(
    ctx: &Ctx,
    server: Option<&str>,
    name: Option<String>,
    json: bool,
    command_args: &[String],
) -> Result<(), CliError> {
    let command = shellops::shell_join(command_args);
    let id = jobops::gen_job_id();
    let meta = JobMeta {
        id: id.clone(),
        name,
        command: command.clone(),
    };
    let meta_json = serde_json::to_string(&meta)
        .map_err(|e| CliError::Usage(format!("cannot encode job metadata: {e}")))?;
    let script = jobops::build_start_script(&id, &meta_json, &command);
    let target = resolve_target(ctx, server).await?;
    let outcome = run_script_on(ctx, &target, &script).await?;
    if outcome.exit_code != 0 {
        return Err(CliError::Usage(format!(
            "job setup failed on server {} (exit {}): {}",
            target.display,
            outcome.exit_code,
            outcome.output.trim()
        )));
    }
    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": id,
                "name": meta.name,
                "log_path": jobops::log_path(&id),
            })
        );
    } else {
        println!("started job {id} on server {}", target.display);
        println!("log: {}", jobops::log_path(&id));
    }
    Ok(())
}
