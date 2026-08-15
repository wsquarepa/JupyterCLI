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
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0]["id"], "aaaaaaaa");
    assert_eq!(jobs[0]["state"], "running");
    assert_eq!(jobs[0]["name"], "vllm");
    assert_eq!(jobs[0]["exit_code"], serde_json::Value::Null);
    assert_eq!(jobs[1]["state"], "exited");
    assert_eq!(jobs[1]["exit_code"], 7);
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
}
