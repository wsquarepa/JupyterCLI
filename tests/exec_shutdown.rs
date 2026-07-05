mod common;

#[path = "common/write_config.rs"]
mod write_config;

#[path = "common/mock_jupyter.rs"]
mod mock_jupyter;

use std::io::Read as _;
use std::process::Stdio;
use std::time::{Duration, Instant};

use mock_jupyter::MockJupyter;
use write_config::write_config;

const HANG_DEADLINE: Duration = Duration::from_secs(15);

#[tokio::test]
async fn exec_exits_promptly_with_stdin_held_open() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &format!("http://{}", mock.addr()));

    let mut child = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["exec", "--", "echo hi"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Keep the child's stdin open: dropping this handle would close fd 0 and unpark the
    // remote read, hiding the runtime-shutdown hang this test exists to catch.
    let _stdin = child.stdin.take().unwrap();

    let deadline = Instant::now() + HANG_DEADLINE;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("jhc hung after exec completed: runtime shutdown blocked on parked stdin read");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    assert!(status.success(), "exec exited with failure: {status:?}");
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(stdout.contains("hi"), "stdout missing 'hi': {stdout:?}");
}
