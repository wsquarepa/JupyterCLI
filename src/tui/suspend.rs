use crate::cli::CliError;

pub fn attach_args(hub: &str, target: &str) -> Vec<String> {
    vec![
        "--hub".to_string(),
        hub.to_string(),
        "shell".to_string(),
        "attach".to_string(),
        target.to_string(),
    ]
}

/// Runs `shell attach` in a separate process with inherited stdio.
///
/// Attach must run in a SEPARATE PROCESS. In-process attach would create two
/// stdin readers (crossterm's event stream and attach's raw passthrough)
/// whose blocking reads race per keystroke, and a parked read would steal
/// input after detach. The subprocess owns stdin exclusively and its runtime
/// shutdown is already handled (commit b552a21).
///
/// The child's status is held unexamined until after raw mode and the
/// alternate screen are re-entered, so a failed child can never leave the
/// app running outside its screen.
pub async fn attach_in_subprocess(hub: &str, target: &str) -> Result<String, CliError> {
    use crossterm::terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    };
    use std::io::Write as _;

    let exe = std::env::current_exe().map_err(CliError::Io)?;

    let mut stdout = std::io::stdout();
    // Clear the visible primary screen so the remote shell starts on a blank
    // page instead of over the user's local shell history. Scrollback is
    // deliberately kept (no ClearType::Purge): only the visible bleed-through
    // is the problem.
    crossterm::execute!(
        stdout,
        LeaveAlternateScreen,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )
    .map_err(CliError::Io)?;
    disable_raw_mode().map_err(CliError::Io)?;
    stdout.flush().map_err(CliError::Io)?;

    let status = tokio::process::Command::new(exe)
        .args(attach_args(hub, target))
        .status()
        .await;

    // The attach session painted the primary screen. Clearing here, still
    // outside the alternate screen, means quitting the TUI later reveals a clean
    // screen instead of remote-shell residue; a session that never attaches
    // leaves the user's screen untouched. Scrollback is kept (no ClearType::Purge).
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )
    .map_err(CliError::Io)?;

    enable_raw_mode().map_err(CliError::Io)?;
    crossterm::execute!(std::io::stdout(), EnterAlternateScreen).map_err(CliError::Io)?;

    let status = status.map_err(CliError::Io)?;
    Ok(if status.success() {
        "attach ended".to_string()
    } else {
        format!("attach exited with {status}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_args_address_shell_through_own_cli() {
        assert_eq!(
            attach_args("icrn", ":2"),
            vec!["--hub", "icrn", "shell", "attach", ":2"]
        );
        assert_eq!(
            attach_args("lab", "backup:1"),
            vec!["--hub", "lab", "shell", "attach", "backup:1"]
        );
    }
}
