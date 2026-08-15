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
    server_started_unix: Option<i64>,
}

async fn resolve_target(ctx: &Ctx, server: Option<&str>) -> Result<JobTarget, CliError> {
    let (entry, display) = server_entry_for(ctx, server).await?;
    let url_path = entry
        .url
        .as_deref()
        .ok_or_else(|| CliError::Usage(format!("server '{display}' reports no URL")))?;
    let client = ServerClient::from_hub(&ctx.client, url_path)?;
    let server_started_unix = entry
        .started
        .as_deref()
        .and_then(jobops::parse_utc_timestamp);
    Ok(JobTarget {
        client,
        display,
        server_started_unix,
    })
}

struct ScriptOutcome {
    output: String,
    exit_code: i32,
}

async fn run_script_to(
    ctx: &Ctx,
    target: &JobTarget,
    script: &str,
    out: &mut impl std::io::Write,
) -> Result<i32, CliError> {
    let name = target.client.create_terminal().await?.name;
    let url = target.client.ws_terminal_url(&name)?;
    let sock = TermSocket::connect(&url, &ctx.hub.effective_token()).await?;
    let result = shellops::exec(sock, script, None, true, out).await;
    // Same belt as exec_cmd: the remote `exit` self-destructs the terminal, DELETE
    // covers error paths and terminado versions that keep exited terminals listed.
    if let Err(cleanup) = target.client.delete_terminal(&name).await {
        eprintln!("warning: could not clean up shell {name}: {cleanup}");
    }
    Ok(result?.exit_code)
}

async fn run_script_on(
    ctx: &Ctx,
    target: &JobTarget,
    script: &str,
) -> Result<ScriptOutcome, CliError> {
    let mut buf: Vec<u8> = Vec::new();
    let exit_code = run_script_to(ctx, target, script, &mut buf).await?;
    Ok(ScriptOutcome {
        output: String::from_utf8_lossy(&buf).into_owned(),
        exit_code,
    })
}

fn job_ref(raw: &str) -> Result<(Option<String>, String), CliError> {
    let (server, id) = parse_shell_ref(raw);
    jobops::validate_job_id(&id).map_err(CliError::Usage)?;
    Ok((server, id))
}

#[derive(serde::Serialize)]
struct JobRow {
    id: String,
    name: Option<String>,
    command: Option<String>,
    state: String,
    exit_code: Option<i32>,
    started_at: Option<String>,
    log_path: String,
}

fn job_row(record: &jobops::ProbeRecord, state: jobops::JobState) -> JobRow {
    JobRow {
        id: record.id.clone(),
        name: record.meta.as_ref().and_then(|m| m.name.clone()),
        command: record.meta.as_ref().map(|m| m.command.clone()),
        state: jobops::state_label(state).to_string(),
        exit_code: record.exit,
        started_at: record.started_at.clone(),
        log_path: jobops::log_path(&record.id),
    }
}

async fn probe(
    ctx: &Ctx,
    target: &JobTarget,
    id: Option<&str>,
) -> Result<Vec<jobops::ProbeRecord>, CliError> {
    let outcome = run_script_on(ctx, target, &jobops::build_probe_script(id)).await?;
    if outcome.exit_code != 0 {
        return Err(CliError::Usage(format!(
            "job probe failed on server {} (exit {}): {}",
            target.display,
            outcome.exit_code,
            outcome.output.trim()
        )));
    }
    Ok(jobops::parse_probe_output(&outcome.output)?)
}

fn not_found(id: &str, display: &str) -> CliError {
    CliError::Usage(format!(
        "job {id} not found on server {display}; run: jhc job list"
    ))
}

async fn list(ctx: &Ctx, server: Option<&str>, json: bool) -> Result<(), CliError> {
    let target = resolve_target(ctx, server).await?;
    let records = probe(ctx, &target, None).await?;
    let rows: Vec<JobRow> = records
        .iter()
        .map(|r| job_row(r, jobops::classify(r, target.server_started_unix)))
        .collect();
    if json {
        println!("{}", serde_json::json!({ "jobs": rows }));
        return Ok(());
    }
    if rows.is_empty() {
        println!("no jobs on server {}", target.display);
        return Ok(());
    }
    println!(
        "{:<10} {:<14} {:<10} {:<6} STARTED",
        "JOB", "NAME", "STATE", "EXIT"
    );
    for row in rows {
        println!(
            "{:<10} {:<14} {:<10} {:<6} {}",
            row.id,
            row.name.as_deref().unwrap_or("-"),
            row.state,
            row.exit_code.map_or("-".to_string(), |c| c.to_string()),
            row.started_at.as_deref().unwrap_or("-"),
        );
    }
    Ok(())
}

async fn status(ctx: &Ctx, job: &str, json: bool) -> Result<(), CliError> {
    let (server, id) = job_ref(job)?;
    let target = resolve_target(ctx, server.as_deref()).await?;
    let records = probe(ctx, &target, Some(id.as_str())).await?;
    let record = records
        .first()
        .ok_or_else(|| not_found(&id, &target.display))?;
    let state = jobops::classify(record, target.server_started_unix);
    let row = job_row(record, state);
    if json {
        println!("{}", serde_json::json!(row));
        return Ok(());
    }
    println!("job {} on server {}", row.id, target.display);
    println!("  name:    {}", row.name.as_deref().unwrap_or("-"));
    println!("  command: {}", row.command.as_deref().unwrap_or("-"));
    println!("  state:   {}", row.state);
    if let Some(code) = row.exit_code {
        println!("  exit:    {code}");
    }
    println!("  started: {}", row.started_at.as_deref().unwrap_or("-"));
    println!("  log:     {}", row.log_path);
    Ok(())
}

async fn tail(
    ctx: &Ctx,
    job: &str,
    follow: bool,
    max_wait: Option<u64>,
    max_bytes: Option<u64>,
) -> Result<(), CliError> {
    let (server, id) = job_ref(job)?;
    let target = resolve_target(ctx, server.as_deref()).await?;
    if follow {
        let script = jobops::build_follow_script(&id, max_wait, max_bytes);
        let code = run_script_to(ctx, &target, &script, &mut std::io::stdout()).await?;
        if code == jobops::TAIL_MISSING_EXIT {
            return Err(not_found(&id, &target.display));
        }
        if !jobops::follow_exit_ok(code) {
            return Err(CliError::Usage(format!(
                "tail failed on server {} (exit {code})",
                target.display
            )));
        }
        return Ok(());
    }
    let bytes = max_bytes.unwrap_or(jobops::DEFAULT_TAIL_BYTES);
    let script = jobops::build_tail_script(&id, bytes);
    let outcome = run_script_on(ctx, &target, &script).await?;
    if outcome.exit_code == jobops::TAIL_MISSING_EXIT {
        return Err(not_found(&id, &target.display));
    }
    if outcome.exit_code != 0 {
        return Err(CliError::Usage(format!(
            "tail failed on server {} (exit {}): {}",
            target.display,
            outcome.exit_code,
            outcome.output.trim()
        )));
    }
    print!("{}", outcome.output);
    Ok(())
}

async fn wait(
    ctx: &Ctx,
    job: &str,
    max_wait: Option<u64>,
    json: bool,
) -> Result<ExitCode, CliError> {
    let (server, id) = job_ref(job)?;
    let started = std::time::Instant::now();
    let budget = max_wait.map(std::time::Duration::from_secs);
    let mut attempt: u32 = 0;
    loop {
        // Re-resolve each poll so a server restart mid-wait updates the generation
        // check instead of leaving a reused pid looking alive forever.
        let target = resolve_target(ctx, server.as_deref()).await?;
        let records = probe(ctx, &target, Some(id.as_str())).await?;
        let Some(record) = records.first() else {
            return Err(not_found(&id, &target.display));
        };
        match jobops::classify(record, target.server_started_unix) {
            jobops::JobState::Exited(code) => {
                if json {
                    println!("{}", serde_json::json!({"id": id, "exit_code": code}));
                }
                return Ok(ExitCode::from(code as u8));
            }
            jobops::JobState::Orphaned => {
                eprintln!(
                    "job {id} is orphaned (its process is gone without an exit code, \
                     likely a server restart); its outcome is unknown"
                );
                return Ok(ExitCode::from(shellops::JHC_FAILURE_EXIT as u8));
            }
            jobops::JobState::Running => {}
        }
        let mut pause = jobops::wait_backoff(attempt);
        attempt += 1;
        if let Some(budget) = budget {
            let elapsed = started.elapsed();
            if elapsed >= budget {
                eprintln!("job {id} still running after {}s", budget.as_secs());
                return Ok(ExitCode::from(shellops::JHC_FAILURE_EXIT as u8));
            }
            pause = pause.min(budget.saturating_sub(elapsed));
        }
        // The probe's exec installs a process-wide SIGINT handler, so once the first
        // poll has run the default "die on Ctrl-C" action is gone; without racing the
        // pause against the signal, Ctrl-C would be swallowed until the next probe.
        tokio::select! {
            _ = tokio::time::sleep(pause) => {}
            _ = tokio::signal::ctrl_c() => {
                eprintln!("job {id} wait interrupted; its outcome is unknown");
                return Ok(ExitCode::from(shellops::JHC_FAILURE_EXIT as u8));
            }
        }
    }
}

async fn kill(ctx: &Ctx, job: &str, force: bool, json: bool) -> Result<(), CliError> {
    let (server, id) = job_ref(job)?;
    let target = resolve_target(ctx, server.as_deref()).await?;
    let records = probe(ctx, &target, Some(id.as_str())).await?;
    let record = records
        .first()
        .ok_or_else(|| not_found(&id, &target.display))?;
    match jobops::classify(record, target.server_started_unix) {
        jobops::JobState::Exited(code) => {
            return Err(CliError::Usage(format!(
                "job {id} already exited with code {code}; nothing to kill"
            )));
        }
        jobops::JobState::Orphaned => {
            return Err(CliError::Usage(format!(
                "job {id} is orphaned (its process is gone); remove it with: jhc job rm {id}"
            )));
        }
        jobops::JobState::Running => {}
    }
    let outcome = run_script_on(ctx, &target, &jobops::build_kill_script(&id, force)).await?;
    if outcome.exit_code != 0 {
        return Err(CliError::Usage(format!(
            "kill failed on server {} (exit {}); the job may have just exited: {}",
            target.display,
            outcome.exit_code,
            outcome.output.trim()
        )));
    }
    let signal = if force { "SIGKILL" } else { "SIGTERM" };
    if json {
        println!("{}", serde_json::json!({"id": id, "signal": signal}));
    } else {
        println!("sent {signal} to job {id} on server {}", target.display);
    }
    Ok(())
}

async fn remove(ctx: &Ctx, job: &str, json: bool) -> Result<(), CliError> {
    let (server, id) = job_ref(job)?;
    let target = resolve_target(ctx, server.as_deref()).await?;
    let records = probe(ctx, &target, Some(id.as_str())).await?;
    let record = records
        .first()
        .ok_or_else(|| not_found(&id, &target.display))?;
    if jobops::classify(record, target.server_started_unix) == jobops::JobState::Running {
        return Err(CliError::Usage(format!(
            "job {id} is running; kill it first with: jhc job kill {id}"
        )));
    }
    let ids = vec![id.clone()];
    let outcome = run_script_on(ctx, &target, &jobops::build_remove_script(&ids)).await?;
    if outcome.exit_code != 0 {
        return Err(CliError::Usage(format!(
            "remove failed on server {} (exit {}): {}",
            target.display,
            outcome.exit_code,
            outcome.output.trim()
        )));
    }
    if json {
        println!("{}", serde_json::json!({"removed": [id]}));
    } else {
        println!("removed job {id} from server {}", target.display);
    }
    Ok(())
}

async fn clean(ctx: &Ctx, server: Option<&str>, json: bool) -> Result<(), CliError> {
    let target = resolve_target(ctx, server).await?;
    let records = probe(ctx, &target, None).await?;
    let finished: Vec<(String, jobops::JobState)> = records
        .iter()
        .filter_map(|r| {
            let state = jobops::classify(r, target.server_started_unix);
            (state != jobops::JobState::Running).then(|| (r.id.clone(), state))
        })
        .collect();
    if finished.is_empty() {
        if json {
            println!("{}", serde_json::json!({"removed": []}));
        } else {
            println!("nothing to clean on server {}", target.display);
        }
        return Ok(());
    }
    let ids: Vec<String> = finished.iter().map(|(id, _)| id.clone()).collect();
    let outcome = run_script_on(ctx, &target, &jobops::build_remove_script(&ids)).await?;
    if outcome.exit_code != 0 {
        return Err(CliError::Usage(format!(
            "clean failed on server {} (exit {}): {}",
            target.display,
            outcome.exit_code,
            outcome.output.trim()
        )));
    }
    if json {
        let removed: Vec<serde_json::Value> = finished
            .iter()
            .map(|(id, state)| serde_json::json!({"id": id, "state": jobops::state_label(*state)}))
            .collect();
        println!("{}", serde_json::json!({"removed": removed}));
    } else {
        for (id, state) in &finished {
            println!("removed job {id} ({})", jobops::state_label(*state));
        }
    }
    Ok(())
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
        JobCmd::List { server, json } => {
            list(ctx, server.as_deref(), json).await?;
            Ok(ExitCode::SUCCESS)
        }
        JobCmd::Status { job, json } => {
            status(ctx, &job, json).await?;
            Ok(ExitCode::SUCCESS)
        }
        JobCmd::Tail {
            job,
            follow,
            max_wait,
            max_bytes,
        } => {
            tail(ctx, &job, follow, max_wait, max_bytes).await?;
            Ok(ExitCode::SUCCESS)
        }
        JobCmd::Wait {
            job,
            max_wait,
            json,
        } => wait(ctx, &job, max_wait, json).await,
        JobCmd::Kill { job, force, json } => {
            kill(ctx, &job, force, json).await?;
            Ok(ExitCode::SUCCESS)
        }
        JobCmd::Rm { job, json } => {
            remove(ctx, &job, json).await?;
            Ok(ExitCode::SUCCESS)
        }
        JobCmd::Clean { server, json } => {
            clean(ctx, server.as_deref(), json).await?;
            Ok(ExitCode::SUCCESS)
        }
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
