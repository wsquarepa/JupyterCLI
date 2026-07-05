use std::time::Duration;

use crate::api::error::ApiError;
use crate::api::ws::{TermFrame, TermSocket};

pub const PEEK_IDLE: Duration = Duration::from_millis(250);

enum StripState {
    Ground,
    Escape,
    Csi,
    Osc,
    OscEsc,
    PendingCr,
}

// A trailing `\r` at the very end of the stream stays buffered in `PendingCr` and is
// dropped when the socket disconnects instead of being flushed. That is a one-character
// ceiling accepted for a strictly chunk-incremental API; upgrading it would require an
// explicit `flush()` call at stream end to decide whether the buffered `\r` becomes `\n`.
pub struct AnsiStripper {
    state: StripState,
}

impl AnsiStripper {
    pub fn new() -> Self {
        Self {
            state: StripState::Ground,
        }
    }

    pub fn push(&mut self, chunk: &str) -> String {
        let mut out = String::with_capacity(chunk.len());
        for ch in chunk.chars() {
            match self.state {
                StripState::Ground => match ch {
                    '\x1b' => self.state = StripState::Escape,
                    '\r' => self.state = StripState::PendingCr,
                    _ => out.push(ch),
                },
                StripState::PendingCr => {
                    out.push('\n');
                    self.state = StripState::Ground;
                    match ch {
                        '\n' => {}
                        '\x1b' => self.state = StripState::Escape,
                        '\r' => self.state = StripState::PendingCr,
                        _ => out.push(ch),
                    }
                }
                StripState::Escape => match ch {
                    '[' => self.state = StripState::Csi,
                    ']' => self.state = StripState::Osc,
                    _ => self.state = StripState::Ground,
                },
                StripState::Csi => {
                    if ('\u{40}'..='\u{7e}').contains(&ch) {
                        self.state = StripState::Ground;
                    }
                }
                StripState::Osc => match ch {
                    '\x07' => self.state = StripState::Ground,
                    '\x1b' => self.state = StripState::OscEsc,
                    _ => {}
                },
                StripState::OscEsc => {
                    self.state = if ch == '\\' {
                        StripState::Ground
                    } else {
                        StripState::Osc
                    };
                }
            }
        }
        out
    }
}

impl Default for AnsiStripper {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn send(mut sock: TermSocket, text: &str) -> Result<(), ApiError> {
    sock.send_stdin(&format!("{text}\n")).await?;
    sock.finish().await
}

pub async fn peek(
    mut sock: TermSocket,
    raw: bool,
    follow: bool,
    out: &mut impl std::io::Write,
) -> Result<(), ApiError> {
    let mut stripper = AnsiStripper::new();
    loop {
        let frame = if follow {
            sock.next_frame().await?
        } else {
            match tokio::time::timeout(PEEK_IDLE, sock.next_frame()).await {
                Ok(frame) => frame?,
                Err(_) => break,
            }
        };
        match frame {
            None => break,
            Some(TermFrame::Stdout(text)) => {
                let rendered = if raw { text } else { stripper.push(&text) };
                out.write_all(rendered.as_bytes())
                    .map_err(|e| ApiError::Protocol {
                        url: "stdout".to_string(),
                        reason: format!("cannot write output: {e}"),
                    })?;
                out.flush().map_err(|e| ApiError::Protocol {
                    url: "stdout".to_string(),
                    reason: format!("cannot flush output: {e}"),
                })?;
            }
            Some(TermFrame::Disconnect) => break,
            Some(TermFrame::Setup) | Some(TermFrame::Unknown(_)) => {}
        }
    }
    sock.finish().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_and_normalizes_crlf() {
        let mut s = AnsiStripper::new();
        let out = s.push("\x1b[?2004hww41@host:~$ ls\r\nfile1\r\nfile2\r\n\x1b[0m");
        assert_eq!(out, "ww41@host:~$ ls\nfile1\nfile2\n");
    }

    #[test]
    fn buffers_escape_split_across_chunks() {
        let mut s = AnsiStripper::new();
        let mut out = s.push("before\x1b[3");
        out.push_str(&s.push("2mafter"));
        assert_eq!(out, "beforeafter");
    }

    #[test]
    fn strips_osc_with_bel_terminator() {
        let mut s = AnsiStripper::new();
        let out = s.push("\x1b]0;window title\x07visible");
        assert_eq!(out, "visible");
    }

    #[test]
    fn lone_cr_becomes_newline() {
        let mut s = AnsiStripper::new();
        assert_eq!(
            s.push("progress 50%\rprogress 100%\r\n"),
            "progress 50%\nprogress 100%\n"
        );
    }
}
