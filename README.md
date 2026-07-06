# JupyterCLI

Manage JupyterHub servers, shells, and files from the terminal. JupyterCLI (`jhc`) talks to the JupyterHub REST API, per-server Jupyter Server APIs, and the terminado WebSocket, so you can start and stop servers, run remote commands, move files, and attach to live shells without leaving your terminal.

`jhc` has two faces: a scriptable subcommand CLI, and an interactive TUI dashboard that opens when you run `jhc` with no subcommand.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/wsquarepa/JupyterCLI/master/install.sh | sh
```

The install script supports Linux and macOS, asks which release channel to install, and writes the `jhc` binary to an existing install location or `~/.local/bin`. See [Installation](docs/install.md) for release downloads, source builds, and custom install locations.

## Quickstart

JupyterCLI needs a hub base URL and an API token. Create a token in the browser at `<hub url>/hub/token`.

1. Run `jhc init` and enter your hub URL and token when prompted.
2. Run `jhc status` to confirm the connection and list your servers.
3. Run `jhc` with no subcommand to open the interactive dashboard.

Non-interactive setup:

```sh
jhc init --url https://jupyter.example.edu --token YOUR_TOKEN
```

The configuration is written to `~/.config/jhc/config.toml` with owner-only permissions.

## Common Commands

```sh
# Interactive dashboard
jhc

# Show the authenticated user and all servers
jhc status

# Start and stop the default server
jhc start
jhc stop

# Run a remote command and exit with its status
jhc exec -- python train.py --epochs 5

# List, copy, and delete remote files
jhc ls :results/
jhc cp :results/out.csv ./out.csv
jhc cp -r ./dataset :data/dataset

# Create a shell and attach to it
jhc shell new
jhc shell attach 1
```

## Documentation

- [Installation](docs/install.md): install script, release assets, source builds, and updates.
- [Usage Guide](docs/usage.md): the interactive dashboard, remote commands, shells, file transfer, presets, and multiple hubs.
- [CLI Reference](docs/cli.md): every subcommand, its flags, the address syntax, and the exec exit-code contract.
- [Configuration](docs/configuration.md): the config file, hub profiles, tokens, spawn presets, and terminal limits.
- [Troubleshooting](docs/troubleshooting.md): authentication errors, spawn failures, lost output, and permissions warnings.

## Highlights

- **Two faces, one tool:** a scriptable subcommand CLI and an interactive ratatui dashboard, sharing the same config and API layer.
- **Remote command execution:** `jhc exec` runs a command over the terminado WebSocket and exits with the remote command's real status code, so it composes in shell pipelines and CI.
- **File transfer:** upload and download single files or whole directories with `jhc cp`, using a familiar `scp`-style `[SERVER:]PATH` address syntax.
- **Live shell attach:** attach to a running terminal interactively and detach with a double `Ctrl-\`, leaving the shell running on the server.
- **Spawn presets:** capture a running server's environment and resource options as a named preset, then start future servers with `jhc start --preset <name>`.
- **Multiple hubs:** keep several named hub profiles in one config and switch per-invocation with `--hub`.

## Contributing

Bug reports and feature requests: [GitHub Issues](https://github.com/wsquarepa/JupyterCLI/issues)

Local development uses the Rust stable toolchain (edition 2024):

```sh
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The full verification suite (fmt check, clippy, tests) is available as `cargo xtask ci`. Run it before submitting changes.

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).
