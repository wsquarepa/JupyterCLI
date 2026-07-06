# Onboarding

This file provides guidance to AI coding agents working with code in this repository.

> This file's canonical location is `.agents/ONBOARDING.md`. `CLAUDE.md` and `AGENTS.md` in the repo root are symlinks to it, so edit `.agents/ONBOARDING.md` and the others follow automatically.

## What this is

JupyterCLI (`jhc`) manages JupyterHub servers, shells, and files from the terminal. It talks to the JupyterHub REST API, per-server Jupyter Server APIs, and the terminado WebSocket. It has two faces: a `clap` subcommand CLI, and an interactive ratatui TUI that opens when `jhc` runs with no subcommand.

## Commands

- Full CI gate (fmt check + clippy + tests): `cargo xtask ci`. Run this before finishing any code change. Automation lives in the `xtask` crate, never in shell scripts.
- Build: `cargo build`
- Run all tests: `cargo test --workspace`
- Single test: `cargo test --workspace <test_name>` (e.g. `cargo test exec_parser_extracts_output_and_code`)
- Single integration file: `cargo test --test cli`
- Lint exactly as CI does: `cargo clippy --workspace --all-targets -- -D warnings`
- Format check: `cargo fmt --all --check`

Edition 2024. `unsafe_code` is forbidden workspace-wide and `#[allow(...)]` attributes are denied by clippy (`allow_attributes = "deny"`), so you cannot silence a lint with an attribute; fix the underlying issue.

## Architecture

### Layering (bottom to top)

1. **`api/`** — HTTP/WS transport. `HubClient` (`api/mod.rs`) hits the hub REST API (`hub/api/...`); `ServerClient` (`api/server.rs`) hits a single server's Jupyter Server API (terminals, contents) and is built `from_hub` by rebasing onto the server's host-absolute `url`. `TermSocket` (`api/ws.rs`) is the terminado WebSocket. `sse.rs` parses the spawn-progress event stream. All request methods live here; `types.rs` holds the wire structs; `error.rs` defines `ApiError` and the shared `check()` that maps non-2xx into actionable errors (e.g. 401 mentions `jhc init`).
2. **`shellops.rs`** — protocol logic over a `TermSocket`, independent of CLI vs TUI. `AnsiStripper` and `ExecParser` are byte-stream state machines that must survive arbitrary chunk boundaries (the tests feed split sentinels deliberately). `exec()` wraps a command in sentinel markers (`\x1e{nonce}:S\x1e` ... `\x1e{nonce}:CODE\x1e`) so it can recover the remote exit code and distinguish real output from the shell's echo. `JHC_FAILURE_EXIT = 125` is the reserved "jhc itself failed" code, kept distinct from any remote status.
3. **`cli/`** — `clap` definitions and dispatch. `cli/mod.rs` holds the `Cli`/`Command` enums, `Ctx::load` (config + client bootstrap, emits a chmod warning on loose perms), option parsing (`key=value` string vs `key:=JSON` typed), and `main()`/`dispatch()`. Each verb group has its own file (`server.rs`, `shell.rs`, `fs.rs`, `token.rs`, `preset.rs`, `init.rs`). `addr.rs` parses the `[SERVER:]PATH` / `[SERVER:]SHELL` address syntax shared across commands.
4. **`tui/`** — the interactive dashboard (see below).

### CLI exit-code contract

`exec` is special: every non-125 exit code belongs to the remote command, so `jhc` must never emit its own generic exit 1 or clap's exit 2 under `exec`. `main()` intercepts clap usage errors targeting `exec` and remaps them to 125; `exit_code_for_failure` does the same for runtime errors. Preserve this when touching error paths.

### TUI architecture (Elm-style)

The TUI is a pure state machine driven by an async event loop, deliberately kept testable without a terminal.

- **`tui/mod.rs`** owns `dashboard_loop`: a `tokio::select!` over crossterm events, an mpsc channel of `AppEvent`s from background tasks, a 15s refresh interval, and a 100ms tick. It drains `app.take_effects()` each iteration and routes each `Effect`.
- **`tui/app.rs`** is `App` — all state and logic. `on_key`/`apply`/`tick` mutate state and enqueue `Effect`s; they never do I/O. This is why nearly the whole file is unit-testable (see its `#[cfg(test)]` block). Async results arrive back as `AppEvent`s via `apply()`.
- **`Effect` routing is split**: *network* effects (`Refresh`, `Start`, `Stop`, terminal CRUD, `PeekStart`) go to `tui/net.rs::dispatch`, which spawns a tokio task that calls the `api` layer and sends an `AppEvent` back. *Local* effects (`Quit`, `PeekStop`, `Attach`, `SavePreset`) are handled inline in `dashboard_loop` because they touch the terminal, the config file, or the peek task handle. `net::dispatch`'s match treats local effects as `unreachable!` — keep the two sets in sync if you add an `Effect`.
- **Op sequencing**: every async operation carries a monotonically increasing `op: u64`. Stale `AppEvent`s (op older than the current one for that slot) are dropped so a slow response can't overwrite newer state. Preserve this when adding operations.
- **Attach runs in a subprocess, not in-process** (`tui/suspend.rs` + `attach.rs`). Two stdin readers (crossterm's event stream and attach's raw passthrough) would race per keystroke, so `attach_in_subprocess` re-execs `jhc shell attach` with inherited stdio, managing the alternate-screen/raw-mode transition around it. Detach is a double `Ctrl-\` (`DETACH_BYTE = 0x1c`) within a 400ms window (`DetachDetector`).
- Supporting modules: `render.rs` (draw only, no state mutation), `dialogs.rs` (modal state machines: start/stop confirmations, create-named-server, preset editor), `wizard.rs` (first-run config wizard shown when no config exists), `grid.rs` (terminal grid layout math), `input.rs` (windowed text input widget), `theme.rs` (fixed dark palette constants, no customization by design).

### Config

`config.rs`: TOML at `$JHC_CONFIG_DIR` or `dirs::config_dir()/jhc/config.toml`, written `0o600` (loose perms trigger a warning). `#[serde(deny_unknown_fields)]` — unknown keys are hard errors. A config has multiple named hubs (each with `url`, `token`, optional `terminal_limit`, and named spawn `presets`) plus a `default_hub`; `--hub` overrides per-invocation. `JUPYTERHUB_API_TOKEN` env var overrides the stored token (`effective_token`).

## Testing conventions

- Logic is unit-tested inline (`#[cfg(test)] mod tests`) right next to the code — `shellops.rs`, `app.rs`, and the `api` modules all follow this. Prefer adding a unit test over an integration test when the logic doesn't need a real binary or socket.
- `api` tests use `wiremock` to mock the hub. TUI tests construct `App` directly and drive `on_key`/`apply` with a fixed `Instant` — no terminal needed.
- Integration tests in `tests/` run the compiled binary end-to-end. `tests/common/mock_jupyter.rs` is a single listener that answers both the REST calls and the terminado WebSocket, so a full `exec` round-trips against it. Use `JHC_CONFIG_DIR` (temp dir) to isolate config.

## Conventions specific to this repo

- Product name is **JupyterCLI**; binary is **jhc**. No em-dashes in any shipped user-facing string (help text, status messages, errors).
- Comments here explain non-obvious protocol/OS constraints (URL rebasing, sentinel chunk-splitting, subprocess attach, exit-code remapping). Match that bar: comment the *why* behind a constraint, never restate code.
- Errors are typed (`thiserror`) and actionable; network calls in `HubClient::get` retry 5xx/transport failures up to 3 times with a warning before raising the last error. Don't add silent fallbacks.
