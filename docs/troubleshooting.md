# Troubleshooting

## Not Configured Yet

If `jhc` reports that it is not configured, run `jhc init` and supply your hub URL and an API token:

```sh
jhc init
jhc init --url https://jupyter.example.edu --token YOUR_TOKEN
```

Create a token in the browser at `<hub url>/hub/token`. The config is written to `~/.config/jhc/config.toml`.

## Token Invalid Or Expired

A `401` from the hub means the token is invalid or expired:

```text
token invalid or expired: run jhc init or check ~/.config/jhc/config.toml
```

Refresh the token by re-running `jhc init` for the same profile (it keeps your presets), or edit `token` in `config.toml`. To inject a token from a secret store for one run, set `JUPYTERHUB_API_TOKEN`:

```sh
JUPYTERHUB_API_TOKEN=abc123... jhc status
```

## Permission Denied For An Operation

A `403` means the token authenticates but lacks the scope for that operation. JupyterHub tokens carry scopes; a read-only token cannot start servers or manage other users' tokens. Create a token with the scopes you need, or ask your hub administrator.

## Intermittent 403 On A Previously Working Token

A `403` whose body does not name a scope (JupyterHub replies with a bare `Forbidden`) means the token itself failed to resolve to an authorized user, not that it lacks a scope. This usually happens when the hub's authenticator requires a fresh browser login on some interval (for example a CILogon hub with `auth_refresh_age`), even though the token is otherwise valid. Sign in to the hub web UI at `<hub url>/hub/home` and retry.

The TUI writes a diagnostic log to `~/.local/state/jhc/logs/jhc-<UTC-timestamp>-<pid>.log` (the exact path is printed when you exit the TUI). For CLI commands, raise verbosity to stderr with `--verbose`, or set `RUST_LOG` for fine-grained control:

```sh
jhc --verbose status
RUST_LOG=jhc=debug jhc status
```

## Config File Permissions Warning

If `jhc` warns that `config.toml` is readable by other users, tighten it:

```sh
chmod 600 ~/.config/jhc/config.toml
```

The file holds your API token, so `jhc` writes it `0600` and warns when it finds looser permissions.

## Cannot List Environment Or Resource Options

JupyterHub does not expose the list of a hub's spawn options (profiles, images, GPUs) over its API, so JupyterCLI cannot enumerate them. Start a server once from `<hub url>/hub/spawn` in the browser, then capture its options as a preset:

```sh
jhc preset import --as my-preset
jhc start --preset my-preset
```

## Server Does Not Expose The Terminals API

A `404` on a terminals request means that server does not expose the Jupyter Server terminals API:

```text
this server does not expose the terminals API
```

The server may not be fully ready, or its image may not include terminal support. Confirm it is running with `jhc status`, and start it if needed with `jhc start`.

## exec Exit Codes Look Wrong

`jhc exec` exits with the remote command's status, so a non-zero exit usually means your remote command failed, not `jhc`. The one exception is `125`: that is the reserved code for when `jhc` itself failed (bad usage, network error, or spawn failure) rather than the remote command. See [the exit-code contract](cli.md#exit-codes).

## Missing Output From A Long Job

`jhc shell peek` shows only the terminal's bounded scrollback, so output from a long-running job can scroll past the ceiling and be lost. Capture the output to a file on the server and fetch it instead:

```sh
jhc shell send 1 -- 'cmd |& tee job.log'
jhc cp :job.log ./job.log
```

## cp Rejects Both Sides

`jhc cp` needs exactly one remote side. It uploads (local to remote) or downloads (remote to local); it does not copy local-to-local or remote-to-remote. Spell the remote side as `[SERVER:]PATH`:

```sh
jhc cp ./out.csv :results/out.csv
jhc cp :results/out.csv ./out.csv
```

If a colon in a local path is being read as a server prefix, note that an argument is remote only when its first `:` comes before its first `/`. Prefix a local path with `./` to keep it local.

## Directory Copy Or Delete Refused

Copying or deleting a directory requires `-r`:

```sh
jhc cp -r ./dataset :data/dataset
jhc rm -r :scratch
```

## Transient Network Errors

`jhc` retries hub read requests that fail with a 5xx or a transport error, up to three attempts, printing a warning before raising the last error. A persistent failure after retries points to the hub being down or unreachable rather than a transient blip. Re-run with `--verbose` to see one summary line per API call:

```sh
jhc --verbose status
```
