# CLI Reference

Run `jhc --help`, or `jhc <command> --help`, for the exact help text for your installed version.

Running `jhc` with no subcommand opens the [interactive dashboard](usage.md#interactive-dashboard).

## Global Options

These apply to every subcommand.

| Option | Description |
|---|---|
| `-V`, `--version` | Print version and exit |
| `--hub <NAME>` | Hub profile from the config to use for this invocation; defaults to `default_hub` |
| `--verbose` | Print one request summary line per API call to stderr |

## Address Syntax

File and shell commands accept an address that optionally names a server:

- `[SERVER:]PATH` for files, e.g. `results/out.csv`, `:results/out.csv`, or `backup:results/out.csv`.
- `[SERVER:]SHELL` for shells, e.g. `1`, `:1`, or `backup:1`.

A path with no server prefix (or an empty prefix like `:out.csv`) targets your default server. A local path in `jhc cp` is any argument whose first `:` comes after the first `/`, so `./weird:name.txt` and `dir/with:colon` stay local. See [Usage](usage.md#addressing-servers-and-shells) for details.

## Setup

### `init`

Create or update the configuration. With no flags it prompts interactively; the token prompt is hidden.

```sh
jhc init
jhc init --url https://jupyter.example.edu --token YOUR_TOKEN
jhc init --url https://jupyter.example.edu --token YOUR_TOKEN --name backup
```

| Option | Description |
|---|---|
| `--url <URL>` | Hub base URL, e.g. `https://jupyter.example.edu` |
| `--token <TOKEN>` | API token for the hub |
| `--name <NAME>` | Name for this hub profile; default `default` |

`init` verifies the token against the hub before saving. Re-running it for an existing profile refreshes the token and URL while preserving that profile's presets. The first profile added becomes the default hub.

## Servers

### `status`

```sh
jhc status
```

Show the authenticated user and all of their servers with state and options.

### `start`

```sh
jhc start
jhc start work
jhc start --preset a100
jhc start -o profile=gpu -o gpus:=2
jhc start --no-wait
```

| Option | Description |
|---|---|
| `server` | Named server to start; omit for the default server |
| `--preset <NAME>` | Spawn preset from the config |
| `-o`, `--option <KV>` | Spawn option `key=value` (string) or `key:=JSON` (typed); repeatable |
| `--no-wait` | Return immediately instead of streaming spawn progress |

Options passed with `-o` override matching keys from `--preset`. See [Configuration](configuration.md#spawn-presets) for the option formats.

### `stop`

```sh
jhc stop
jhc stop work
```

Stop the default server, or a named server.

## Presets

### `preset import`

```sh
jhc preset import
jhc preset import work --as gpu-box
```

Capture a running server's spawn options and save them as a preset. JupyterHub does not expose the list of available environment and resource options over its API, so importing from a server you started once in the browser is how you seed presets.

| Option | Description |
|---|---|
| `server` | Server whose options to capture; omit for the default server |
| `--as <NAME>` | Preset name to save under; default `imported` |

### `preset list`

```sh
jhc preset list
```

List the presets configured for the active hub.

## Shells

`jhc shell` manages terminado shells on a server. A shell is addressed as `[SERVER:]SHELL`.

### `shell new`

```sh
jhc shell new
jhc shell new work
```

Create a shell on a server and print its id.

### `shell list`

```sh
jhc shell list
jhc shell list work
```

List the shells on a server.

### `shell send`

```sh
jhc shell send 1 -- python train.py --epochs 5
jhc shell send backup:1 -- 'cmd |& tee job.log'
```

Write a command line to a shell and return immediately. A newline is appended. Text after `--` is sent verbatim, so quoting and argument boundaries are preserved.

### `shell peek`

```sh
jhc shell peek 1
jhc shell peek 1 --follow
jhc shell peek 1 --raw
```

Print a shell's recent output.

| Option | Description |
|---|---|
| `--follow` | Keep streaming live output until interrupted |
| `--raw` | Emit verbatim bytes instead of stripped text |

`peek` shows only the terminal's bounded scrollback, so output from a long-running job can scroll past the ceiling and be lost. For long jobs, capture to a file and fetch it:

```sh
jhc shell send 1 -- 'cmd |& tee job.log'
jhc cp :job.log ./job.log
```

### `shell attach`

```sh
jhc shell attach 1
jhc shell attach backup:1
```

Attach interactively to a shell. Press `Ctrl-\` twice within a short window to detach; the shell keeps running on the server.

### `shell kill`

```sh
jhc shell kill 1
```

Destroy a shell.

## Remote Commands

### `exec`

```sh
jhc exec -- python train.py --epochs 5
jhc exec work -- ls -la
jhc exec --shell 1 -- tail -f log.txt
```

Run a command on a server and exit with its status. By default `exec` uses an ephemeral shell; pass `--shell [SERVER:]SHELL` to reuse an existing one.

| Option | Description |
|---|---|
| `server` | Server to run on; omit for the default server |
| `--shell <SHELL>` | Existing shell to reuse instead of an ephemeral one, as `[SERVER:]SHELL` |
| `command` (after `--`) | Command and arguments to run |

Arguments after `--` are run as an argv, not a shell line, so no top-level shell interpretation happens. For pipes, redirection, or globbing, invoke a shell yourself:

```sh
jhc exec -- bash -c 'ls | grep foo'
```

#### Exit codes

`exec` is designed to compose in scripts and pipelines, so its exit code is the remote command's status. `jhc` never emits its own generic exit `1` or a clap usage exit `2` under `exec`. When `jhc` itself fails (bad usage, network error, spawn failure) rather than the remote command, it exits with the reserved code `125`, kept distinct from any real remote status.

## Files

### `ls`

```sh
jhc ls :results/
jhc ls backup:data
```

List remote files at `[SERVER:]PATH`.

### `cp`

```sh
jhc cp :results/out.csv ./out.csv
jhc cp ./dataset.zip :data/dataset.zip
jhc cp -r ./dataset :data/dataset
jhc cp -r backup:runs ./runs
```

Copy files between the local machine and a server. Exactly one side must be remote: `cp` uploads (local to remote) or downloads (remote to local), never local-to-local or remote-to-remote.

| Option | Description |
|---|---|
| `src` | Source: local path or `[SERVER:]PATH` |
| `dst` | Destination: local path or `[SERVER:]PATH` |
| `-r`, `--recursive` | Copy directories recursively |

Copying a directory without `-r` is an error.

### `rm`

```sh
jhc rm :old-output.txt
jhc rm -r :scratch
```

Delete a remote file or directory.

| Option | Description |
|---|---|
| `path` | Remote path as `[SERVER:]PATH` |
| `-r`, `--recursive` | Delete directories recursively |

Deleting a directory without `-r` is an error.

## Tokens

`jhc token` manages your hub API tokens.

### `token list`

```sh
jhc token list
```

### `token create`

```sh
jhc token create
jhc token create --note "laptop"
```

Create a token and print it. The default note is `created by JupyterCLI`.

### `token revoke`

```sh
jhc token revoke <id>
```

Revoke a token by the id shown in `token list`.
