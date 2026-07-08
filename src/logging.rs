//! Logging setup and redaction helpers. CLI runs log to stderr gated by
//! `RUST_LOG`; the TUI logs to an auto-named file because stderr writes would
//! corrupt the alternate screen. Token material and terminal payload bytes are
//! never recorded; a token appears only as a fingerprint.

use time::OffsetDateTime;
use time::macros::format_description;

pub const BODY_SNIPPET_CHARS: usize = 2000;

/// Default `RUST_LOG`-style directive used only when `RUST_LOG` is unset in CLI
/// mode. `--verbose` raises jhc targets to debug; third-party stays at warn.
pub fn cli_directive(verbose: bool) -> &'static str {
    if verbose { "warn,jhc=debug" } else { "warn" }
}

/// Default directive for the TUI file sink when `RUST_LOG` is unset: capture all
/// jhc instrumentation, keep third-party crates at warn.
pub const TUI_DIRECTIVE: &str = "warn,jhc=debug";

/// `jhc-YYYYMMDDTHHMMSSZ-<pid>.log`. UTC keeps names sortable and unambiguous
/// across timezones; the pid disambiguates same-second launches.
pub fn log_file_name(now: OffsetDateTime, pid: u32) -> String {
    let fmt = format_description!("[year][month][day]T[hour][minute][second]Z");
    let stamp = now
        .to_offset(time::UtcOffset::UTC)
        .format(&fmt)
        .unwrap_or_else(|_| "unknown".to_string());
    format!("jhc-{stamp}-{pid}.log")
}

/// FNV-1a fingerprint so log lines can tell which token was in use without
/// recording token material. Non-cryptographic, but a 64-bit preimage of a
/// high-entropy secret is not recoverable from a local debug artifact.
pub fn fingerprint(secret: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in secret.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Truncate on a char boundary so multibyte responses cannot panic the logger.
pub fn snippet(body: &str, max_chars: usize) -> String {
    body.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn fingerprint_is_stable_and_discriminating() {
        assert_eq!(fingerprint("abc"), fingerprint("abc"));
        assert_ne!(fingerprint("abc"), fingerprint("abd"));
        assert_eq!(fingerprint("abc").len(), 16);
    }

    #[test]
    fn snippet_respects_char_boundaries() {
        assert_eq!(snippet("héllo wörld", 6), "héllo ");
        assert_eq!(snippet("short", 2000), "short");
    }

    #[test]
    fn log_file_name_formats_utc_and_pid() {
        let now = datetime!(2026-07-08 14:30:00 UTC);
        assert_eq!(log_file_name(now, 12345), "jhc-20260708T143000Z-12345.log");
    }

    #[test]
    fn cli_directive_switches_on_verbose() {
        assert_eq!(cli_directive(false), "warn");
        assert_eq!(cli_directive(true), "warn,jhc=debug");
    }
}
