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

pub const JHC_FAILURE_EXIT: i32 = 125;

pub fn build_exec_line(command: &str, nonce: &str) -> String {
    format!(
        "stty -echo; printf '\\036{nonce}:S\\036'; {{ {command}; }}; printf '\\036{nonce}:%d\\036' $?; exit\n"
    )
}

enum ExecState {
    BeforeStart,
    Streaming,
    Done,
}

// Longest suffix of `buf` that equals a proper prefix of `needle`, in bytes.
// A full match is handled by the caller's `find`, so the overlap is capped at `needle.len() - 1`.
fn sentinel_overlap(buf: &str, needle: &str) -> usize {
    let b = buf.as_bytes();
    let n = needle.as_bytes();
    let max = b.len().min(n.len().saturating_sub(1));
    (1..=max)
        .rev()
        .find(|&k| b[b.len() - k..] == n[..k])
        .unwrap_or(0)
}

pub struct ExecParser {
    start_marker: String,
    end_prefix: String,
    state: ExecState,
    buf: String,
    pending_cr: bool,
}

impl ExecParser {
    pub fn new(nonce: &str) -> Self {
        Self {
            start_marker: format!("\x1e{nonce}:S\x1e"),
            end_prefix: format!("\x1e{nonce}:"),
            state: ExecState::BeforeStart,
            buf: String::new(),
            pending_cr: false,
        }
    }

    pub fn push(&mut self, chunk: &str) -> (String, Option<i32>) {
        self.buf.push_str(chunk);
        let mut out = String::new();
        loop {
            match self.state {
                ExecState::BeforeStart => {
                    if let Some(pos) = self.buf.find(&self.start_marker) {
                        self.buf.drain(..pos + self.start_marker.len());
                        self.state = ExecState::Streaming;
                    } else {
                        // Retain only the buffer suffix that could be the start of a split
                        // start sentinel. The marker is pure ASCII, so any overlapping suffix
                        // is ASCII and `buf.len() - keep` lands on a char boundary; a byte-count
                        // retention would panic on multibyte pre-start output.
                        let keep = sentinel_overlap(&self.buf, &self.start_marker);
                        self.buf.drain(..self.buf.len() - keep);
                        return (out, None);
                    }
                }
                ExecState::Streaming => {
                    if let Some(pos) = self.buf.find(&self.end_prefix) {
                        out.push_str(&normalize_crlf(&self.buf[..pos], &mut self.pending_cr));
                        self.buf.drain(..pos + self.end_prefix.len());
                        if let Some(end) = self.buf.find('\x1e') {
                            // A malformed number between two valid sentinel delimiters can only be
                            // produced by the shell itself misbehaving; 125 is the defined
                            // "JupyterCLI could not determine the outcome" code, not a silent fallback.
                            let code: i32 = self.buf[..end].parse().unwrap_or(JHC_FAILURE_EXIT);
                            self.state = ExecState::Done;
                            return (out, Some(code));
                        }
                        // End marker seen but the code digits have not fully arrived;
                        // restore the prefix and wait for more bytes.
                        self.buf.insert_str(0, &self.end_prefix);
                        return (out, None);
                    }
                    // Retain only the buffer suffix that could be the start of a split end
                    // sentinel; emit everything before it. `end_prefix` is ASCII, so any
                    // overlapping suffix is ASCII and `buf.len() - keep` lands on a char boundary.
                    let keep = sentinel_overlap(&self.buf, &self.end_prefix);
                    let emit: String = self.buf.drain(..self.buf.len() - keep).collect();
                    out.push_str(&normalize_crlf(&emit, &mut self.pending_cr));
                    return (out, None);
                }
                ExecState::Done => return (out, None),
            }
        }
    }
}

// Normalizes CRLF and lone CR to LF, carrying a `\r` that lands at a chunk boundary in
// `pending_cr` so a following `\n` collapses instead of producing a blank line.
fn normalize_crlf(text: &str, pending_cr: &mut bool) -> String {
    let mut result = String::with_capacity(text.len());
    for ch in text.chars() {
        if *pending_cr {
            *pending_cr = false;
            result.push('\n');
            if ch == '\n' {
                continue;
            }
        }
        if ch == '\r' {
            *pending_cr = true;
        } else {
            result.push(ch);
        }
    }
    result
}

pub struct ExecOutcome {
    pub exit_code: i32,
}

pub async fn exec(
    mut sock: TermSocket,
    command: &str,
    stdin_pipe: Option<tokio::io::Stdin>,
    out: &mut impl std::io::Write,
) -> Result<ExecOutcome, ApiError> {
    use rand::RngExt as _;
    use tokio::io::AsyncReadExt as _;

    let nonce: String = {
        let mut rng = rand::rng();
        (0..16)
            .map(|_| format!("{:x}", rng.random_range(0..16u8)))
            .collect()
    };
    let mut parser = ExecParser::new(&nonce);
    sock.send_stdin(&build_exec_line(command, &nonce)).await?;

    let mut stdin = stdin_pipe;
    let mut stdin_buf = [0u8; 8192];
    loop {
        tokio::select! {
            frame = sock.next_frame() => {
                match frame? {
                    None => {
                        return Err(ApiError::Protocol {
                            url: "terminal".to_string(),
                            reason: "connection closed before the command finished".to_string(),
                        });
                    }
                    Some(TermFrame::Stdout(text)) => {
                        let (output, code) = parser.push(&text);
                        out.write_all(output.as_bytes()).map_err(|e| ApiError::Protocol {
                            url: "stdout".to_string(),
                            reason: format!("cannot write output: {e}"),
                        })?;
                        out.flush().ok();
                        if let Some(code) = code {
                            sock.finish().await?;
                            return Ok(ExecOutcome { exit_code: code });
                        }
                    }
                    Some(_) => {}
                }
            }
            read = async {
                match stdin.as_mut() {
                    Some(pipe) => pipe.read(&mut stdin_buf).await,
                    None => std::future::pending().await,
                }
            } => {
                match read {
                    Ok(0) => {
                        sock.send_stdin("\x04").await?;
                        stdin = None;
                    }
                    Ok(n) => {
                        sock.send_stdin(&String::from_utf8_lossy(&stdin_buf[..n])).await?;
                    }
                    Err(e) => {
                        return Err(ApiError::Protocol {
                            url: "stdin".to_string(),
                            reason: format!("cannot read piped stdin: {e}"),
                        });
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                sock.send_stdin("\x03").await?;
            }
        }
    }
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

    #[test]
    fn exec_parser_extracts_output_and_code() {
        let n = "abcd1234";
        let mut p = ExecParser::new(n);
        let (out1, code1) = p.push("echoed command line junk\r\n");
        assert_eq!((out1.as_str(), code1), ("", None));
        let (out2, code2) = p.push(&format!("\x1e{n}:S\x1ehello\r\nworld\r\n\x1e{n}:0\x1e"));
        assert_eq!(out2, "hello\nworld\n");
        assert_eq!(code2, Some(0));
    }

    #[test]
    fn exec_parser_handles_sentinels_split_across_chunks() {
        let n = "abcd1234";
        let mut p = ExecParser::new(n);
        let first = format!("\x1e{n}");
        let (out1, _) = p.push(&first);
        assert_eq!(out1, "");
        let (out2, _) = p.push(":S\x1epartial");
        assert_eq!(out2, "partial");
        let (out3, code) = p.push(&format!(" done\x1e{n}:13"));
        assert_eq!(out3, " done");
        assert_eq!(code, None);
        let (out4, code4) = p.push("\x1e");
        assert_eq!(out4, "");
        assert_eq!(code4, Some(13));
    }

    #[test]
    fn exec_parser_is_not_spoofed_by_echoed_printf() {
        let n = "abcd1234";
        let mut p = ExecParser::new(n);
        let echoed = format!("printf '\\036{n}:S\\036'; {{ ls; }}\r\n");
        let (out, code) = p.push(&echoed);
        assert_eq!((out.as_str(), code), ("", None));
    }

    #[test]
    fn exec_parser_multibyte_pre_start_output_does_not_panic() {
        let n = "abcd1234";
        let mut p = ExecParser::new(n);
        // 24 bytes of 2-byte chars: the old byte-count retention drained to a
        // non-char-boundary index and panicked.
        let (out1, code1) = p.push(&"\u{e9}".repeat(12));
        assert_eq!((out1.as_str(), code1), ("", None));
        let (out2, code2) = p.push(&format!("\x1e{n}:S\x1eok\r\n\x1e{n}:0\x1e"));
        assert_eq!(out2, "ok\n");
        assert_eq!(code2, Some(0));
    }

    #[test]
    fn exec_line_shape() {
        let line = build_exec_line("nvidia-smi", "abcd1234");
        assert!(line.starts_with("stty -echo; printf"));
        assert!(line.contains("{ nvidia-smi; }"));
        assert!(line.ends_with("; exit\n"));
    }
}
