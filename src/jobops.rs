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
// bash before the exit sentinel prints.
pub fn build_start_script(id: &str, meta_json: &str, command: &str) -> String {
    let runner = format!(
        "echo $$ > \"$0/pid\"; {{ {command}; }} > \"$0/log\" 2>&1; \
         echo $? > \"$0/exit.tmp\" && mv \"$0/exit.tmp\" \"$0/exit\""
    );
    format!(
        "dir=\"{JOBS_DIR}/{id}\"; \
         if mkdir -p \"$dir\" && printf '%s' {meta} > \"$dir/meta.json\" && \
         date -u +%FT%TZ > \"$dir/started\"; \
         then setsid bash -c {script} \"$dir\" & else false; fi",
        meta = shell_quote(meta_json),
        script = shell_quote(&runner),
    )
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
        assert!(script.ends_with("\"$dir\" & else false; fi"));
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
}
