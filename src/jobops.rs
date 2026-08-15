use crate::shellops::shell_quote;

pub const JOB_ID_LEN: usize = 8;
// $HOME stays unexpanded here; every generated script wraps it in double quotes and
// the remote shell expands it. Home is the one path guaranteed writable and
// persistent on JupyterHub pods.
const JOBS_DIR: &str = "$HOME/.jhc/jobs";

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
    format!("~/.jhc/jobs/{id}/log")
}

// The runner records its own $$ rather than the launcher capturing $!, because
// setsid(1) forks when its caller is already a process group leader, in which case
// $! names the wrong process. The exit code is written to exit.tmp then renamed so a
// reader never observes a half-written file. The if/else keeps a setup failure
// visible in the exec exit status without `exit`, which would kill the terminado
// bash before the exit sentinel prints. The subshell around the launch matters: the
// interactive terminado bash would otherwise print a job-control notice for the
// background job between exec's sentinels.
pub fn build_start_script(id: &str, meta_json: &str, command: &str) -> String {
    let runner = format!(
        "echo $$ > \"$0/pid\"; {{ {command}; }} < /dev/null > \"$0/log\" 2>&1; \
         echo $? > \"$0/exit.tmp\" && mv \"$0/exit.tmp\" \"$0/exit\""
    );
    format!(
        "dir=\"{JOBS_DIR}/{id}\"; \
         if mkdir -p \"$dir\" && printf '%s' {meta} > \"$dir/meta.json\" && \
         date -u +%FT%TZ > \"$dir/started\"; \
         then ( setsid bash -c {script} \"$dir\" & ) else false; fi",
        meta = shell_quote(meta_json),
        script = shell_quote(&runner),
    )
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error(
        "malformed job probe record: expected 5 fields, got {fields} ({line_len} bytes); the remote ~/.jhc/jobs state may be damaged"
    )]
    MalformedRecord { fields: usize, line_len: usize },
    #[error(
        "job '{id}' has unreadable metadata: {reason}; recreate it or remove it with: jhc job rm {id}"
    )]
    BadMeta { id: String, reason: String },
}

#[derive(Debug, Clone)]
pub struct ProbeRecord {
    pub id: String,
    pub exit: Option<i32>,
    pub pid_alive: bool,
    pub started_at: Option<String>,
    pub meta: Option<JobMeta>,
}

// One line per job, fields separated by the ASCII unit separator (0x1f, printf \037):
// id, exit code or -, pid-alive 1/0, started timestamp or -, meta.json base64 or -.
// base64 keeps arbitrary metadata bytes from ever colliding with the delimiter.
pub fn build_probe_script(id: Option<&str>) -> String {
    let glob = match id {
        Some(id) => format!("\"{JOBS_DIR}/{id}/\""),
        None => format!("\"{JOBS_DIR}\"/*/"),
    };
    format!(
        "for d in {glob}; do [ -d \"$d\" ] || continue; \
         if [ -f \"$d/exit\" ]; then ec=$(cat \"$d/exit\"); else ec=-; fi; \
         if [ -f \"$d/pid\" ] && kill -0 \"$(cat \"$d/pid\")\" 2>/dev/null; then alive=1; else alive=0; fi; \
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
        let fields: Vec<&str> = line.split('\x1f').collect();
        let [id, exit, alive, started, meta] = fields.as_slice() else {
            return Err(JobError::MalformedRecord {
                fields: fields.len(),
                line_len: line.len(),
            });
        };
        let exit = match *exit {
            "-" => None,
            digits => Some(
                digits
                    .parse::<i32>()
                    .map_err(|_| JobError::MalformedRecord {
                        fields: fields.len(),
                        line_len: line.len(),
                    })?,
            ),
        };
        let pid_alive = match *alive {
            "1" => true,
            "0" => false,
            _ => {
                return Err(JobError::MalformedRecord {
                    fields: fields.len(),
                    line_len: line.len(),
                });
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
        assert!(script.contains("mv \"$0/exit.tmp\" \"$0/exit\""));
        assert!(script.ends_with("\"$dir\" & ) else false; fi"));
        assert!(script.contains("< /dev/null > \"$0/log\" 2>&1"));
        assert_bash_parses(&script);
    }

    #[test]
    fn start_script_survives_hostile_command_bytes() {
        let nasty = "'echo' 'a'\\''b'ne'wline\n$(reboot)'";
        assert_bash_parses(&build_start_script("aabbccdd", "{}", nasty));
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

    #[test]
    fn probe_output_rejects_malformed_records_loudly() {
        assert!(matches!(
            parse_probe_output("only-two\x1ffields\n"),
            Err(JobError::MalformedRecord { fields: 2, .. })
        ));
        assert!(matches!(
            parse_probe_output("aabbccdd\x1fnot-a-code\x1f1\x1f-\x1f-\n"),
            Err(JobError::MalformedRecord { .. })
        ));
        assert!(matches!(
            parse_probe_output("aabbccdd\x1f-\x1f1\x1f-\x1f!!notb64!!\n"),
            Err(JobError::BadMeta { .. })
        ));
    }

    #[test]
    fn probe_output_empty_means_no_jobs() {
        assert!(parse_probe_output("").unwrap().is_empty());
        assert!(parse_probe_output("\n").unwrap().is_empty());
    }
}
