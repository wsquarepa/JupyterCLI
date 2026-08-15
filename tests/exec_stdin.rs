mod common;

#[path = "common/write_config.rs"]
mod write_config;

#[path = "common/mock_jupyter.rs"]
mod mock_jupyter;

use std::io::Write as _;
use std::process::Stdio;

use mock_jupyter::MockJupyter;
use write_config::write_config;

/// Pins the documented exec stdin contract: piped stdin is forwarded to the remote
/// command byte for byte, and EOF is delivered as Ctrl-D, so `jhc exec -- bash -s`
/// runs a local script remotely as one unit.
#[tokio::test]
async fn exec_forwards_piped_stdin_and_signals_eof_as_ctrl_d() {
    let mock = MockJupyter::spawn().await;
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), &format!("http://{}", mock.addr()));

    let mut child = common::client_bin()
        .env("JHC_CONFIG_DIR", dir.path())
        .env_remove("JUPYTERHUB_API_TOKEN")
        .args(["exec", "--", "bash", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"echo from-stdin\n")
        .unwrap();

    // MockJupyter serves on this test's current-thread runtime, so the wait must yield:
    // a blocking wait_with_output here starves the mock and deadlocks both processes.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("jhc hung: stdin forwarding or the exit sentinel never completed");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("got:echo from-stdin"),
        "piped script bytes never reached the remote command: {stdout:?}"
    );
}
