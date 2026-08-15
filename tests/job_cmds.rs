mod common;

#[path = "common/write_config.rs"]
mod write_config;

#[path = "common/mock_jupyter.rs"]
mod mock_jupyter;

use mock_jupyter::MockJupyter;
use write_config::write_config;

async fn jhc(mock: &MockJupyter, dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    write_config(dir, &format!("http://{}", mock.addr()));
    let mut cmd = common::client_bin();
    cmd.env("JHC_CONFIG_DIR", dir)
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(args);
    // MockJupyter serves on this test's runtime, so the wait must yield: a blocking
    // output() on this thread starves the mock and deadlocks both processes.
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn job_start_prints_id_and_log_path() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(
        &mock,
        dir.path(),
        &["job", "start", "--name", "burn", "--", "sleep", "999"],
    )
    .await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let id_line = stdout.lines().next().unwrap();
    assert!(id_line.starts_with("started job "), "stdout: {stdout}");
    let id = id_line
        .trim_start_matches("started job ")
        .split(' ')
        .next()
        .unwrap();
    assert_eq!(id.len(), 8);
    assert!(stdout.contains(&format!("~/.jhc/jobs/{id}/log")));
}

#[tokio::test]
async fn job_start_json_reports_id_name_and_log_path() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(
        &mock,
        dir.path(),
        &[
            "job", "start", "--json", "--name", "burn", "--", "sleep", "999",
        ],
    )
    .await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(payload["name"], "burn");
    assert_eq!(payload["id"].as_str().unwrap().len(), 8);
    assert!(payload["log_path"].as_str().unwrap().ends_with("/log"));
}

#[tokio::test]
async fn job_rejects_malformed_ids_before_any_network_io() {
    // Valid config, unreachable hub: the id check in job_ref must fail before any
    // network call, or this test times out against port 9 instead.
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "http://127.0.0.1:9");
    let out = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["job", "status", "NOT-AN-ID"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("not a job id"), "stderr: {stderr}");
}

#[tokio::test]
async fn job_list_json_classifies_running_and_exited() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "list", "--json"]).await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let jobs = payload["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0]["id"], "aaaaaaaa");
    assert_eq!(jobs[0]["state"], "running");
    assert_eq!(jobs[0]["name"], "vllm");
    assert_eq!(jobs[0]["exit_code"], serde_json::Value::Null);
    assert_eq!(jobs[1]["state"], "exited");
    assert_eq!(jobs[1]["exit_code"], 7);
    assert_eq!(jobs[2]["id"], "cccccccc");
    assert_eq!(jobs[2]["state"], "orphaned");
    assert_eq!(jobs[2]["exit_code"], serde_json::Value::Null);
}

#[tokio::test]
async fn job_list_human_output_is_a_table() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "list"]).await;
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("JOB"), "header missing: {stdout}");
    assert!(stdout.contains("aaaaaaaa") && stdout.contains("running"));
    assert!(stdout.contains("bbbbbbbb") && stdout.contains("exited"));
}

#[tokio::test]
async fn job_status_shows_one_job_and_404s_actionably() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "status", "aaaaaaaa"]).await;
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("running") && stdout.contains("'sleep' '999'"),
        "stdout: {stdout}"
    );

    let missing = jhc(&mock, dir.path(), &["job", "status", "eeeeeeee"]).await;
    assert!(!missing.status.success());
    let stderr = String::from_utf8(missing.stderr).unwrap();
    assert!(
        stderr.contains("not found") && stderr.contains("jhc job list"),
        "stderr: {stderr}"
    );
}

#[tokio::test]
async fn job_tail_prints_log_bytes_and_follow_budget_exits_zero() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "tail", "aaaaaaaa"]).await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("job log line")
    );

    let follow = jhc(
        &mock,
        dir.path(),
        &["job", "tail", "aaaaaaaa", "--follow", "--max-wait", "1"],
    )
    .await;
    assert_eq!(follow.status.code(), Some(0));
    assert!(
        String::from_utf8(follow.stdout)
            .unwrap()
            .contains("job log line")
    );
}

#[tokio::test]
async fn job_tail_missing_job_is_an_actionable_error() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "tail", "eeeeeeee"]).await;
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("not found"), "stderr: {stderr}");

    let follow = jhc(&mock, dir.path(), &["job", "tail", "eeeeeeee", "--follow"]).await;
    assert!(!follow.status.success());
    let stderr = String::from_utf8(follow.stderr).unwrap();
    assert!(stderr.contains("not found"), "stderr: {stderr}");
}

#[tokio::test]
async fn job_tail_rejects_zero_max_wait() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(
        &mock,
        dir.path(),
        &["job", "tail", "aaaaaaaa", "--follow", "--max-wait", "0"],
    )
    .await;
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("1..") || stderr.contains("not in"),
        "stderr: {stderr}"
    );
}

#[tokio::test]
async fn job_wait_propagates_the_remote_exit_code() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "wait", "bbbbbbbb"]).await;
    assert_eq!(
        out.status.code(),
        Some(7),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn job_wait_json_reports_the_exit_code() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "wait", "bbbbbbbb", "--json"]).await;
    assert_eq!(out.status.code(), Some(7));
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(payload["id"], "bbbbbbbb");
    assert_eq!(payload["exit_code"], 7);
}

#[tokio::test]
async fn job_wait_missing_job_exits_125() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "wait", "eeeeeeee"]).await;
    assert_eq!(out.status.code(), Some(125));
    assert!(String::from_utf8(out.stderr).unwrap().contains("not found"));
}

#[tokio::test]
async fn job_wait_orphaned_exits_125() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "wait", "cccccccc"]).await;
    assert_eq!(out.status.code(), Some(125));
    assert!(String::from_utf8(out.stderr).unwrap().contains("orphaned"));
}

#[tokio::test]
async fn job_wait_times_out_with_125_while_running() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(
        &mock,
        dir.path(),
        &["job", "wait", "aaaaaaaa", "--max-wait", "1"],
    )
    .await;
    assert_eq!(
        out.status.code(),
        Some(125),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("still running")
    );
}

#[tokio::test]
async fn job_wait_ctrl_c_interrupts_promptly_with_125() {
    // The first probe registers a process-wide SIGINT handler inside exec, so a
    // plain sleep between polls would swallow every later Ctrl-C.
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &format!("http://{}", mock.addr()));
    let mut cmd = common::client_bin();
    cmd.env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["job", "wait", "aaaaaaaa", "--max-wait", "20"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let out = tokio::task::spawn_blocking(move || {
        let child = cmd.spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let signalled = std::process::Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(signalled.success());
        let started = std::time::Instant::now();
        let out = child.wait_with_output().unwrap();
        (out, started.elapsed())
    })
    .await
    .unwrap();
    let (out, took) = out;
    assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
    assert_eq!(
        out.status.code(),
        Some(125),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("interrupted"), "stderr: {stderr}");
}

#[tokio::test]
async fn job_wait_huge_max_wait_does_not_panic() {
    // The deadline math must never add a Duration to an Instant: that panics on
    // overflow (exit 101) and would break the "125 or the remote code" contract.
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(
        &mock,
        dir.path(),
        &[
            "job",
            "wait",
            "bbbbbbbb",
            "--max-wait",
            "18446744073709551615",
        ],
    )
    .await;
    assert_eq!(
        out.status.code(),
        Some(7),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn job_wait_transport_failure_exits_125() {
    // Config points at a closed port: Ctx loads fine, the hub call fails.
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "http://127.0.0.1:9");
    let out = tokio::task::spawn_blocking(move || {
        common::client_bin()
            .env("JHC_CONFIG_DIR", dir.path())
            .env_remove("JUPYTERHUB_API_TOKEN")
            .args(["job", "wait", "aaaaaaaa"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        out.status.code(),
        Some(125),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn job_kill_signals_running_and_refuses_exited() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "kill", "aaaaaaaa"]).await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8(out.stdout).unwrap().contains("SIGTERM"));

    let exited = jhc(&mock, dir.path(), &["job", "kill", "bbbbbbbb"]).await;
    assert!(!exited.status.success());
    let stderr = String::from_utf8(exited.stderr).unwrap();
    assert!(
        stderr.contains("already exited") && stderr.contains("7"),
        "stderr: {stderr}"
    );
}

#[tokio::test]
async fn job_rm_refuses_running_and_removes_exited() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let running = jhc(&mock, dir.path(), &["job", "rm", "aaaaaaaa"]).await;
    assert!(!running.status.success());
    let stderr = String::from_utf8(running.stderr).unwrap();
    assert!(
        stderr.contains("running") && stderr.contains("jhc job kill"),
        "stderr: {stderr}"
    );

    let exited = jhc(&mock, dir.path(), &["job", "rm", "bbbbbbbb"]).await;
    assert!(
        exited.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&exited.stderr)
    );
    assert!(
        String::from_utf8(exited.stdout)
            .unwrap()
            .contains("removed job bbbbbbbb")
    );
}

#[tokio::test]
async fn job_rm_json_matches_the_clean_shape() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "rm", "bbbbbbbb", "--json"]).await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        payload,
        serde_json::json!({"removed": [{"id": "bbbbbbbb", "state": "exited"}]})
    );
}

#[tokio::test]
async fn job_wait_json_reports_orphaned_and_running_states() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let orphaned = jhc(&mock, dir.path(), &["job", "wait", "cccccccc", "--json"]).await;
    assert_eq!(orphaned.status.code(), Some(125));
    let payload: serde_json::Value = serde_json::from_slice(&orphaned.stdout).unwrap();
    assert_eq!(
        payload,
        serde_json::json!({"id": "cccccccc", "state": "orphaned"})
    );
    assert!(
        String::from_utf8(orphaned.stderr)
            .unwrap()
            .contains("orphaned")
    );

    let running = jhc(
        &mock,
        dir.path(),
        &["job", "wait", "aaaaaaaa", "--max-wait", "1", "--json"],
    )
    .await;
    assert_eq!(running.status.code(), Some(125));
    let payload: serde_json::Value = serde_json::from_slice(&running.stdout).unwrap();
    assert_eq!(
        payload,
        serde_json::json!({"id": "aaaaaaaa", "state": "running"})
    );
    assert!(
        String::from_utf8(running.stderr)
            .unwrap()
            .contains("still running")
    );
}

#[tokio::test]
async fn job_clean_json_lists_removed_ids_with_states() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "clean", "--json"]).await;
    assert!(out.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        payload,
        serde_json::json!({"removed": [
            {"id": "bbbbbbbb", "state": "exited"},
            {"id": "cccccccc", "state": "orphaned"},
        ]})
    );
}

#[tokio::test]
async fn job_clean_removes_only_finished_jobs() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    let out = jhc(&mock, dir.path(), &["job", "clean"]).await;
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("bbbbbbbb"), "stdout: {stdout}");
    assert!(stdout.contains("cccccccc"), "stdout: {stdout}");
    assert!(stdout.contains("orphaned"), "stdout: {stdout}");
    assert!(
        !stdout.contains("aaaaaaaa"),
        "must not touch the running job: {stdout}"
    );
}
