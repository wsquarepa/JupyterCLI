use crate::shellops::shell_quote;

pub const JOB_ID_LEN: usize = 8;
// Relative to the remote home. Scripts spell it as "$HOME/<rel>" (unexpanded here, the
// remote shell expands it inside double quotes) and user-facing paths as "~/<rel>". Home
// is the one path guaranteed writable and persistent on JupyterHub pods.
const JOBS_DIR_REL: &str = ".jhc/jobs";
const GENERATION_SLACK_SECS: i64 = 60;
const WAIT_BACKOFF_CEILING_SECS: u64 = 15;
pub const DEFAULT_TAIL_BYTES: u64 = 65536;
// Reserved by the tail scripts for "no such job log": log content cannot forge an
// exit status, so this stays unambiguous where a sentinel string in output would not.
pub const TAIL_MISSING_EXIT: i32 = 66;

pub fn gen_job_id() -> String {
    use rand::RngExt as _;
    let mut rng = rand::rng();
    (0..JOB_ID_LEN)
        .map(|_| format!("{:x}", rng.random_range(0..16u8)))
        .collect()
}

// Ids are embedded verbatim in generated shell scripts, so this validation is the
// injection guard for user-supplied ids: anything outside lowercase hex is rejected
// before it can reach a script.
pub fn validate_job_id(id: &str) -> Result<(), String> {
    let ok = id.len() == JOB_ID_LEN && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "'{id}' is not a job id ({JOB_ID_LEN} lowercase hex chars); run: jhc job list"
        ))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobMeta {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub command: String,
}

pub fn log_path(id: &str) -> String {
    format!("~/{JOBS_DIR_REL}/{id}/log")
}

// The runner records its own $$ rather than the launcher capturing $!, because
// setsid(1) forks when its caller is already a process group leader, in which case
// $! names the wrong process. The exit code is written to exit.tmp then renamed so a
// reader never observes a half-written file. The if/else keeps a setup failure
// visible in the exec exit status without `exit`, which would kill the terminado
// bash before the exit sentinel prints. The subshell around the launch matters: the
// interactive terminado bash would otherwise print a job-control notice for the
// background job between exec's sentinels.
// The launcher redirects the whole runner, not just the command, so nothing in the job
// keeps the ephemeral pty open and a runner failure (a mv that cannot write the exit
// file) lands in the log instead of a dead pty.
// The runner traps TERM with a no-op handler rather than ignoring it: a handler is
// reset to default across exec, so the job command still dies on the group SIGTERM
// that job kill sends, while the runner survives to record its exit code; SIG_IGN
// would be inherited by the command.
pub fn build_start_script(id: &str, meta_json: &str, command: &str) -> String {
    let runner = format!(
        "trap ':' TERM; echo $$ > \"$0/pid\"; {{ {command}; }}; \
         echo $? > \"$0/exit.tmp\" && mv \"$0/exit.tmp\" \"$0/exit\""
    );
    format!(
        "dir=\"$HOME/{JOBS_DIR_REL}/{id}\"; \
         if mkdir -p \"$dir\" && printf '%s' {meta} > \"$dir/meta.json\" && \
         date -u +%FT%TZ > \"$dir/started\"; \
         then ( setsid bash -c {script} \"$dir\" < /dev/null > \"$dir/log\" 2>&1 & ) else false; fi",
        meta = shell_quote(meta_json),
        script = shell_quote(&runner),
    )
}

// The body runs in a subshell so `exit 66` leaves only that subshell and the code
// flows to `$?`; exiting the ephemeral terminado bash itself would drop the exec
// end sentinel and jhc would see a closed connection instead of the code.
fn tail_body(id: &str, cmd: &str) -> String {
    format!(
        "( log=\"$HOME/{JOBS_DIR_REL}/{id}/log\"; [ -f \"$log\" ] || exit {TAIL_MISSING_EXIT}; {cmd} )"
    )
}

pub fn build_tail_script(id: &str, max_bytes: u64) -> String {
    tail_body(id, &format!("tail -c {max_bytes} \"$log\""))
}

// --max-bytes doubles as the initial window and the stream cap, so tail never emits
// more than the budget. `timeout` wraps tail, not the pipeline: on expiry head sees
// EOF and the pipeline exits 0; a plain timeout expiry without head exits 124. The
// prompt return once head's budget fills relies on GNU tail noticing the closed pipe;
// an older coreutils lingers until the next log write or the timeout.
pub fn build_follow_script(id: &str, max_wait: Option<u64>, max_bytes: Option<u64>) -> String {
    let window = max_bytes.unwrap_or(DEFAULT_TAIL_BYTES);
    let mut cmd = format!("tail -c {window} -f \"$log\"");
    if let Some(secs) = max_wait {
        cmd = format!("timeout {secs} {cmd}");
    }
    if let Some(bytes) = max_bytes {
        cmd = format!("{cmd} | head -c {bytes}");
    }
    tail_body(id, &cmd)
}

// 124 is timeout's expiry without head, 141 is tail dying of SIGPIPE once head has its
// budget; a Ctrl-C never reaches the remote tail because jhc cancels the exec instead.
pub fn follow_exit_ok(code: i32) -> bool {
    matches!(code, 0 | 124 | 141)
}

// The caller must have classified the job as Running first (generation check
// included); this script trusts the pid file it finds.
pub fn build_kill_script(id: &str, force: bool) -> String {
    let sig = if force { "KILL" } else { "TERM" };
    format!("d=\"$HOME/{JOBS_DIR_REL}/{id}\"; kill -{sig} -- -\"$(cat \"$d/pid\")\"")
}

pub fn build_remove_script(ids: &[String]) -> String {
    let dirs = ids
        .iter()
        .map(|id| format!("\"$HOME/{JOBS_DIR_REL}/{id}\""))
        .collect::<Vec<_>>()
        .join(" ");
    format!("rm -rf -- {dirs}")
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error(
        "malformed job probe record ({line_len} bytes): {reason}; the remote ~/.jhc/jobs state may be damaged"
    )]
    MalformedRecord { reason: String, line_len: usize },
    #[error(
        "job directory '{id}' under ~/.jhc/jobs is not a job id ({JOB_ID_LEN} lowercase hex chars); remove it by hand"
    )]
    BadId { id: String },
    #[error(
        "job '{id}' has unreadable metadata: {reason}; recreate it or remove it with: jhc job rm {id}"
    )]
    BadMeta { id: String, reason: String },
    #[error("job {id} not found on server {server}; run: jhc job list")]
    NotFound { id: String, server: String },
    #[error("{verb} failed on server {server} (exit {exit_code}): {output}")]
    RemoteFailed {
        verb: &'static str,
        server: String,
        exit_code: i32,
        output: String,
    },
    #[error(
        "kill failed on server {server} (exit {exit_code}); the job may have just exited: {output}"
    )]
    KillFailed {
        server: String,
        exit_code: i32,
        output: String,
    },
}

#[derive(Debug, Clone)]
pub struct ProbeRecord {
    pub id: String,
    pub exit: Option<i32>,
    pub pid_alive: bool,
    pub started_at: Option<String>,
    pub meta: Option<JobMeta>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Exited(i32),
    Orphaned,
}

pub fn state_label(state: JobState) -> &'static str {
    match state {
        JobState::Running => "running",
        JobState::Exited(_) => "exited",
        JobState::Orphaned => "orphaned",
    }
}

pub fn parse_utc_timestamp(raw: &str) -> Option<i64> {
    time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|dt| dt.unix_timestamp())
}

// The generation check closes the pid-reuse hole after a pod recycle: a live pid can
// belong to an unrelated process. The slack absorbs pod/hub clock skew and biases
// uncertainty toward Running, because misreporting a live job as orphaned lets
// `job clean` destroy a live job's log.
pub fn classify(record: &ProbeRecord, server_started_unix: Option<i64>) -> JobState {
    if let Some(code) = record.exit {
        return JobState::Exited(code);
    }
    if !record.pid_alive {
        return JobState::Orphaned;
    }
    let job_started = record.started_at.as_deref().and_then(|raw| {
        let parsed = parse_utc_timestamp(raw);
        if parsed.is_none() {
            tracing::debug!(
                target: "jhc::job",
                id = %record.id,
                started_at = %raw,
                "job start timestamp is unparsable; generation check skipped"
            );
        }
        parsed
    });
    if let (Some(job), Some(server)) = (job_started, server_started_unix)
        && job + GENERATION_SLACK_SECS < server
    {
        return JobState::Orphaned;
    }
    JobState::Running
}

pub fn wait_backoff(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_secs((1u64 << attempt.min(4)).min(WAIT_BACKOFF_CEILING_SECS))
}

// The all-jobs glob relies on bash's default globbing: with no jobs the pattern stays
// literal, fails the -d test, and the loop emits nothing. A remote bashrc that sets
// failglob makes the probe exit non-zero instead, which surfaces as a loud probe
// failure rather than a false "no jobs". A pid file is trusted only when it is all
// digits without a leading zero: kill -0 on -1, 0, or an empty string succeeds against
// the caller's own group and would report a dead job alive.
// One line per job, fields separated by the ASCII unit separator (0x1f, printf \037):
// id, exit code or -, pid-alive 1/0, started timestamp or -, meta.json base64 or -.
// base64 keeps arbitrary metadata bytes from ever colliding with the delimiter.
pub fn build_probe_script(id: Option<&str>) -> String {
    let glob = match id {
        Some(id) => format!("\"$HOME/{JOBS_DIR_REL}/{id}/\""),
        None => format!("\"$HOME/{JOBS_DIR_REL}\"/*/"),
    };
    format!(
        "for d in {glob}; do [ -d \"$d\" ] || continue; \
         if [ -f \"$d/exit\" ]; then ec=$(cat \"$d/exit\"); else ec=-; fi; \
         if [ -f \"$d/pid\" ]; then pid=$(cat \"$d/pid\"); else pid=; fi; \
         case \"$pid\" in ''|0*|*[!0-9]*) alive=0;; *) if kill -0 \"$pid\" 2>/dev/null; then alive=1; else alive=0; fi;; esac; \
         if [ -f \"$d/started\" ]; then st=$(cat \"$d/started\"); else st=-; fi; \
         if [ -f \"$d/meta.json\" ]; then meta=$(base64 -w0 < \"$d/meta.json\"); else meta=-; fi; \
         printf '%s\\037%s\\037%s\\037%s\\037%s\\n' \"$(basename \"$d\")\" \"$ec\" \"$alive\" \"$st\" \"$meta\"; \
         done"
    )
}

pub fn parse_probe_output(text: &str) -> Result<Vec<ProbeRecord>, JobError> {
    use base64::Engine as _;
    let mut records = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let malformed = |reason: String| JobError::MalformedRecord {
            reason,
            line_len: line.len(),
        };
        let fields: Vec<&str> = line.split('\x1f').collect();
        let [id, exit, alive, started, meta] = fields.as_slice() else {
            return Err(malformed(format!(
                "expected 5 fields, got {}",
                fields.len()
            )));
        };
        // Directory names feed rm -rf and kill scripts, so a stray non-job entry under
        // the jobs dir must be refused here rather than trusted downstream.
        if validate_job_id(id).is_err() {
            return Err(JobError::BadId { id: id.to_string() });
        }
        let exit = match *exit {
            "-" => None,
            digits => {
                let code = digits
                    .parse::<i32>()
                    .ok()
                    .filter(|code| (0..=255).contains(code))
                    .ok_or_else(|| {
                        malformed(format!("exit code {digits:?} is not an integer in 0..=255"))
                    })?;
                Some(code)
            }
        };
        let pid_alive = match *alive {
            "1" => true,
            "0" => false,
            other => {
                return Err(malformed(format!(
                    "pid-alive flag {other:?} is neither 0 nor 1"
                )));
            }
        };
        let started_at = (*started != "-").then(|| started.to_string());
        let meta = match *meta {
            "-" => None,
            encoded => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|e| JobError::BadMeta {
                        id: id.to_string(),
                        reason: format!("invalid base64: {e}"),
                    })?;
                Some(
                    serde_json::from_slice::<JobMeta>(&bytes).map_err(|e| JobError::BadMeta {
                        id: id.to_string(),
                        reason: format!("invalid JSON: {e}"),
                    })?,
                )
            }
        };
        records.push(ProbeRecord {
            id: id.to_string(),
            exit,
            pid_alive,
            started_at,
            meta,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Runs `bash -n` (parse only, execute nothing) over a generated script. CI and dev
    // machines are Linux with bash, same assumption the repo's unix-permission tests make.
    fn assert_bash_parses(script: &str) {
        let status = std::process::Command::new("bash")
            .args(["-n", "-c", script])
            .status()
            .expect("bash must be runnable in tests");
        assert!(status.success(), "bash rejected script: {script}");
    }

    fn record(exit: Option<i32>, pid_alive: bool, started_at: Option<&str>) -> ProbeRecord {
        ProbeRecord {
            id: "aabbccdd".to_string(),
            exit,
            pid_alive,
            started_at: started_at.map(String::from),
            meta: None,
        }
    }

    #[test]
    fn job_ids_are_lowercase_hex_of_fixed_length() {
        for _ in 0..64 {
            let id = gen_job_id();
            assert_eq!(id.len(), JOB_ID_LEN);
            assert!(id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        }
    }

    #[test]
    fn job_id_validation_rejects_shell_metacharacters() {
        assert!(validate_job_id("aabbccdd").is_ok());
        for bad in [
            "",
            "AABBCCDD",
            "aabbccd",
            "aabbccddd",
            "../../etc",
            "a; rm -rf",
        ] {
            let err = validate_job_id(bad).unwrap_err();
            assert!(
                err.contains("jhc job list"),
                "error must be actionable: {err}"
            );
        }
    }

    #[test]
    fn start_script_embeds_quoted_meta_and_command_and_parses() {
        let meta = r#"{"id":"aabbccdd","name":"vllm","command":"'sleep' '99'"}"#;
        let script = build_start_script("aabbccdd", meta, "'sleep' '99'");
        assert!(script.starts_with("dir=\"$HOME/.jhc/jobs/aabbccdd\";"));
        assert!(script.contains(&crate::shellops::shell_quote(meta)));
        assert!(script.contains("date -u +%FT%TZ > \"$dir/started\""));
        assert!(script.contains("setsid bash -c"));
        assert!(script.contains("echo $$ > \"$0/pid\""));
        assert!(script.contains("trap '\\'':'\\'' TERM; echo $$ > \"$0/pid\""));
        assert!(script.contains("mv \"$0/exit.tmp\" \"$0/exit\""));
        assert!(script.ends_with("\"$dir\" < /dev/null > \"$dir/log\" 2>&1 & ) else false; fi"));
        assert!(!script.contains("$0/log"));
        assert_bash_parses(&script);
    }

    #[test]
    fn start_script_survives_hostile_command_bytes() {
        let nasty = "'echo' 'a'\\''b'ne'wline\n$(reboot)'";
        assert_bash_parses(&build_start_script("aabbccdd", "{}", nasty));
    }

    // Runs a generated script the way `jhc` does remotely: through the ephemeral exec
    // line under a real bash, with `$HOME` pointed at a scratch dir. Returns the exit
    // code recovered from the end sentinel, or None when the shell died before it.
    fn run_ephemeral(script: &str, home: &std::path::Path) -> (String, Option<i32>) {
        let nonce = "abcd1234";
        let output = std::process::Command::new("bash")
            .args(["-c", &crate::shellops::build_exec_line(script, nonce, true)])
            .env("HOME", home)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .expect("bash must be runnable in tests");
        let mut parser = crate::shellops::ExecParser::new(nonce);
        parser.push(&String::from_utf8_lossy(&output.stdout))
    }

    fn wait_for_file(path: &std::path::Path) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn start_probe_and_remove_round_trip_through_real_bash() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".jhc/jobs/aabbccdd");
        let meta = r#"{"id":"aabbccdd","name":null,"command":"'true'"}"#;

        let (_, code) = run_ephemeral(&build_start_script("aabbccdd", meta, "'true'"), home.path());
        assert_eq!(code, Some(0));
        wait_for_file(&dir.join("exit"));
        for file in ["pid", "started", "meta.json", "log"] {
            assert!(dir.join(file).is_file(), "missing {file}");
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("exit")).unwrap().trim(),
            "0"
        );

        let (output, code) = run_ephemeral(&build_probe_script(Some("aabbccdd")), home.path());
        assert_eq!(code, Some(0), "probe output: {output:?}");
        let records = parse_probe_output(&output).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "aabbccdd");
        assert_eq!(records[0].exit, Some(0));
        assert_eq!(records[0].meta.as_ref().unwrap().command, "'true'");

        let (_, code) = run_ephemeral(&build_remove_script(&["aabbccdd".to_string()]), home.path());
        assert_eq!(code, Some(0));
        assert!(!dir.exists());
    }

    #[test]
    fn probe_treats_non_numeric_pid_files_as_dead() {
        // `kill -0 -1` and `kill -0 0` signal the caller's own group and succeed, so a
        // damaged pid file must never read as alive; a real live pid must.
        let home = tempfile::tempdir().unwrap();
        let mut expected = Vec::new();
        for (id, pid, expect_alive) in [
            ("aabbccd0", "0".to_string(), false),
            ("aabbccd1", "-1".to_string(), false),
            ("aabbccd2", String::new(), false),
            ("aabbccd3", "12abc".to_string(), false),
            ("aabbccd4", std::process::id().to_string(), true),
        ] {
            let dir = home.path().join(".jhc/jobs").join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("pid"), pid).unwrap();
            expected.push((id.to_string(), expect_alive));
        }
        let (output, code) = run_ephemeral(&build_probe_script(None), home.path());
        assert_eq!(code, Some(0), "probe output: {output:?}");
        let records = parse_probe_output(&output).unwrap();
        let alive: Vec<(String, bool)> = records
            .iter()
            .map(|r| (r.id.clone(), r.pid_alive))
            .collect();
        assert_eq!(alive, expected);
    }

    #[test]
    fn tail_scripts_report_a_missing_log_through_the_sentinel() {
        // `exit 66` must leave only a subshell: exiting the ephemeral bash itself
        // would drop the end sentinel and surface as a closed connection instead.
        let home = tempfile::tempdir().unwrap();
        let (_, code) = run_ephemeral(&build_tail_script("aabbccdd", 4096), home.path());
        assert_eq!(code, Some(TAIL_MISSING_EXIT));
        let (_, code) = run_ephemeral(&build_follow_script("aabbccdd", Some(1), None), home.path());
        assert_eq!(code, Some(TAIL_MISSING_EXIT));
    }

    #[test]
    fn tail_script_bounds_bytes_and_reserves_missing_exit() {
        let script = build_tail_script("aabbccdd", 4096);
        assert!(script.starts_with("( log=\"$HOME/.jhc/jobs/aabbccdd/log\";"));
        assert!(script.contains("tail -c 4096 \"$log\""));
        assert!(script.contains("|| exit 66;"));
        assert!(script.ends_with(" )"));
        assert_bash_parses(&script);
    }

    #[test]
    fn follow_script_composes_window_timeout_and_byte_budget() {
        let bare = build_follow_script("aabbccdd", None, None);
        assert!(bare.contains(&format!("tail -c {DEFAULT_TAIL_BYTES} -f \"$log\"")));
        assert!(!bare.contains("timeout") && !bare.contains("head"));

        let waited = build_follow_script("aabbccdd", Some(30), None);
        assert!(waited.contains("timeout 30 tail -c"));

        let both = build_follow_script("aabbccdd", Some(30), Some(9000));
        assert!(both.contains("timeout 30 tail -c 9000 -f \"$log\" | head -c 9000"));
        for script in [&bare, &waited, &both] {
            assert!(script.starts_with("( log=\"$HOME/.jhc/jobs/aabbccdd/log\";"));
            assert!(script.contains("|| exit 66;"));
            assert!(script.ends_with(" )"));
            assert_bash_parses(script);
        }
    }

    #[test]
    fn follow_exit_codes_for_budget_are_success() {
        for ok in [0, 124, 141] {
            assert!(follow_exit_ok(ok));
        }
        for bad in [1, 2, 66, 127, 130] {
            assert!(!follow_exit_ok(bad));
        }
    }

    #[test]
    fn kill_script_signals_the_process_group() {
        let term = build_kill_script("aabbccdd", false);
        assert!(term.contains("kill -TERM -- -\"$(cat \"$d/pid\")\""));
        let kill = build_kill_script("aabbccdd", true);
        assert!(kill.contains("kill -KILL"));
        assert_bash_parses(&term);
        assert_bash_parses(&kill);
    }

    #[test]
    fn remove_script_deletes_each_named_job_dir() {
        let script = build_remove_script(&["aabbccdd".to_string(), "11223344".to_string()]);
        assert_eq!(
            script,
            "rm -rf -- \"$HOME/.jhc/jobs/aabbccdd\" \"$HOME/.jhc/jobs/11223344\""
        );
        assert_bash_parses(&script);
    }

    #[test]
    fn log_path_names_the_job_log() {
        assert_eq!(log_path("aabbccdd"), "~/.jhc/jobs/aabbccdd/log");
    }

    #[test]
    fn probe_script_globs_all_jobs_or_one_and_parses() {
        let all = build_probe_script(None);
        assert!(all.contains("\"$HOME/.jhc/jobs\"/*/"));
        let one = build_probe_script(Some("aabbccdd"));
        assert!(one.contains("\"$HOME/.jhc/jobs/aabbccdd/\""));
        for script in [&all, &one] {
            assert!(script.contains("base64 -w0"));
            assert_bash_parses(script);
        }
    }

    #[test]
    fn probe_output_parses_records_and_absent_fields() {
        use base64::Engine as _;
        let meta = base64::engine::general_purpose::STANDARD
            .encode(r#"{"id":"aabbccdd","name":"vllm","command":"'sleep' '99'"}"#);
        let text = format!(
            "aabbccdd\x1f-\x1f1\x1f2026-08-14T10:00:00Z\x1f{meta}\n\
             11223344\x1f7\x1f0\x1f-\x1f-\n\n"
        );
        let records = parse_probe_output(&text).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id, "aabbccdd");
        assert_eq!(records[0].exit, None);
        assert!(records[0].pid_alive);
        assert_eq!(
            records[0].started_at.as_deref(),
            Some("2026-08-14T10:00:00Z")
        );
        assert_eq!(
            records[0].meta.as_ref().unwrap().name.as_deref(),
            Some("vllm")
        );
        assert_eq!(records[1].exit, Some(7));
        assert!(!records[1].pid_alive);
        assert!(records[1].started_at.is_none());
        assert!(records[1].meta.is_none());
    }

    fn malformed_reason(text: &str) -> String {
        match parse_probe_output(text) {
            Err(JobError::MalformedRecord { reason, .. }) => reason,
            other => panic!("expected MalformedRecord, got {other:?}"),
        }
    }

    #[test]
    fn probe_output_rejects_malformed_records_loudly() {
        assert!(malformed_reason("only-two\x1ffields\n").contains("expected 5 fields, got 2"));
        assert!(malformed_reason("aabbccdd\x1fnot-a-code\x1f1\x1f-\x1f-\n").contains("exit code"));
        assert!(malformed_reason("aabbccdd\x1f0\x1fmaybe\x1f-\x1f-\n").contains("pid-alive"));
        assert!(matches!(
            parse_probe_output("aabbccdd\x1f-\x1f1\x1f-\x1f!!notb64!!\n"),
            Err(JobError::BadMeta { .. })
        ));
        let err = parse_probe_output("only-two\x1ffields\n").unwrap_err();
        assert!(
            err.to_string().contains("expected 5 fields, got 2")
                && err.to_string().contains("~/.jhc/jobs"),
            "message: {err}"
        );
    }

    #[test]
    fn probe_output_rejects_exit_codes_outside_a_byte() {
        for bad in ["-1", "256", "99999"] {
            let reason = malformed_reason(&format!("aabbccdd\x1f{bad}\x1f0\x1f-\x1f-\n"));
            assert!(reason.contains("0..=255"), "reason: {reason}");
        }
        let records = parse_probe_output("aabbccdd\x1f255\x1f0\x1f-\x1f-\n").unwrap();
        assert_eq!(records[0].exit, Some(255));
    }

    #[test]
    fn probe_output_rejects_directory_names_that_are_not_job_ids() {
        // Record ids feed rm -rf and kill scripts, so a stray directory under
        // ~/.jhc/jobs must be refused before it can reach any of them.
        for bad in ["$(reboot)", "AABBCCDD", "aabbccd", "../etc"] {
            match parse_probe_output(&format!("{bad}\x1f-\x1f1\x1f-\x1f-\n")) {
                Err(JobError::BadId { id }) => {
                    assert_eq!(id, bad);
                }
                other => panic!("expected BadId for {bad:?}, got {other:?}"),
            }
        }
        let err = parse_probe_output("$(reboot)\x1f-\x1f1\x1f-\x1f-\n").unwrap_err();
        assert!(
            err.to_string().contains("remove it by hand"),
            "message: {err}"
        );
    }

    #[test]
    fn probe_output_empty_means_no_jobs() {
        assert!(parse_probe_output("").unwrap().is_empty());
        assert!(parse_probe_output("\n").unwrap().is_empty());
    }

    #[test]
    fn timestamps_parse_rfc3339_with_and_without_fraction() {
        assert_eq!(parse_utc_timestamp("1970-01-01T00:01:00Z"), Some(60));
        assert_eq!(parse_utc_timestamp("1970-01-01T00:01:00.500000Z"), Some(60));
        assert_eq!(parse_utc_timestamp("not a time"), None);
        assert_eq!(parse_utc_timestamp("-"), None);
    }

    #[test]
    fn classification_prefers_exit_file_then_pid_then_generation() {
        // exit file wins even over a live pid
        assert_eq!(
            classify(&record(Some(7), true, None), None),
            JobState::Exited(7)
        );
        // dead or missing pid means orphaned
        assert_eq!(
            classify(&record(None, false, None), None),
            JobState::Orphaned
        );
        // live pid, no timestamps: running
        assert_eq!(classify(&record(None, true, None), None), JobState::Running);
    }

    #[test]
    fn generation_check_orphans_stale_jobs_with_slack_toward_running() {
        let job = record(None, true, Some("1970-01-01T00:01:00Z")); // unix 60
        // server started 61s after the job: beyond slack, pid must be reused
        assert_eq!(classify(&job, Some(121)), JobState::Orphaned);
        // exactly at the slack boundary: still running (bias toward running)
        assert_eq!(classify(&job, Some(120)), JobState::Running);
        // server older than the job: running
        assert_eq!(classify(&job, Some(0)), JobState::Running);
        // unparsable job timestamp: check skipped, running
        let odd = record(None, true, Some("gibberish"));
        assert_eq!(classify(&odd, Some(9_999_999)), JobState::Running);
    }

    #[test]
    fn wait_backoff_doubles_to_a_fifteen_second_ceiling() {
        let secs: Vec<u64> = (0..7).map(|a| wait_backoff(a).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 15, 15, 15]);
    }

    #[test]
    fn remote_failure_messages_are_actionable() {
        let not_found = JobError::NotFound {
            id: "aabbccdd".to_string(),
            server: "ww41".to_string(),
        };
        assert_eq!(
            not_found.to_string(),
            "job aabbccdd not found on server ww41; run: jhc job list"
        );
        let failed = JobError::RemoteFailed {
            verb: "tail",
            server: "ww41".to_string(),
            exit_code: 2,
            output: "tail: no such file".to_string(),
        };
        assert_eq!(
            failed.to_string(),
            "tail failed on server ww41 (exit 2): tail: no such file"
        );
        let kill = JobError::KillFailed {
            server: "ww41".to_string(),
            exit_code: 1,
            output: "kill: no such process".to_string(),
        };
        assert!(kill.to_string().contains("the job may have just exited"));
        assert!(
            kill.to_string()
                .starts_with("kill failed on server ww41 (exit 1)")
        );
    }

    #[test]
    fn state_labels_are_stable_json_values() {
        assert_eq!(state_label(JobState::Running), "running");
        assert_eq!(state_label(JobState::Exited(7)), "exited");
        assert_eq!(state_label(JobState::Orphaned), "orphaned");
    }
}
