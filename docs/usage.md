# Usage Guide

JupyterCLI has two faces that share the same config and API layer: a scriptable subcommand CLI, and an interactive dashboard. This guide covers both. For an exhaustive flag list, see the [CLI Reference](cli.md).

## First Run

Configure a hub before anything else:

```sh
jhc init
```

Enter your hub base URL and an API token when prompted. Create a token in the browser at `<hub url>/hub/token`. To set up without prompts:

```sh
jhc init --url https://jupyter.example.edu --token YOUR_TOKEN
```

`init` verifies the token against the hub before writing `~/.config/jhc/config.toml`. Confirm the connection with:

```sh
jhc status
```

## Interactive Dashboard

Run `jhc` with no subcommand to open the dashboard. It lists your servers on the left; selecting a running server shows its terminals in a grid.

If no config exists yet, the dashboard opens a first-run wizard that walks you through the same setup as `jhc init`.

### Controls

| Key | Context | Action |
|---|---|---|
| Tab | Global | Switch focus between the server list and the terminal grid |
| r | Global | Refresh |
| q | Global | Quit |
| Up/Down | Server list | Move the server cursor |
| Enter | Server list | Open the selected running server |
| n | Server list | Start a server (opens the start dialog) |
| x | Server list | Stop the selected server (with confirmation) |
| Arrows | Terminal grid | Move the grid cursor |
| Enter | Terminal grid | Attach to the hovered terminal |
| n | Terminal grid | Create a new terminal |
| x | Terminal grid | Kill the hovered terminal (with confirmation) |

Hovering a terminal in the grid peeks its recent output live without attaching. The start dialog lets you pick a preset, edit spawn options, and save the current options as a new preset.

### Attach and detach

Attaching to a terminal takes over your terminal for an interactive session. Attach runs in a subprocess (it re-execs `jhc shell attach`) so keystrokes are not split between the dashboard and the shell. Press `Ctrl-\` twice within a short window to detach; the shell keeps running on the server.

## Managing Servers From The CLI

```sh
jhc status
jhc start
jhc start work
jhc stop
```

`start` streams spawn progress until the server is ready. Pass `--no-wait` to return immediately. Supply spawn options with a preset, inline options, or both:

```sh
jhc start --preset a100
jhc start -o profile=gpu -o gpus:=2
jhc start --preset a100 -o gpus:=4
```

Inline `-o` options override matching keys from the preset. String options use `key=value`; typed (JSON) options use `key:=JSON`, so `gpus:=2` sends a number and `debug:=true` sends a boolean. See [Configuration](configuration.md#spawn-presets).

## Spawn Presets

JupyterHub does not expose the list of a hub's environment and resource options over its API, so JupyterCLI cannot enumerate them for you. Instead, start a server once from `<hub url>/hub/spawn` in the browser, then capture its options:

```sh
jhc preset import --as gpu-box
jhc preset list
jhc start --preset gpu-box
```

You can also save a preset from the dashboard's start dialog.

## Remote Commands

`jhc exec` runs a command on a server and exits with the remote command's status, so it drops into scripts and pipelines:

```sh
jhc exec -- python train.py --epochs 5
jhc exec work -- ls -la
```

Everything after `--` is an argv, not a shell line: quoting and argument boundaries are preserved and there is no top-level shell interpretation. For pipes, redirection, or globbing, run a shell yourself:

```sh
jhc exec -- bash -c 'ls | grep foo'
```

By default `exec` uses a fresh ephemeral shell. Reuse an existing one with `--shell`:

```sh
jhc exec --shell 1 -- tail -n 100 log.txt
```

When the remote command fails, `jhc` exits with its status. When `jhc` itself fails, it exits with the reserved code `125`. See [the exit-code contract](cli.md#exit-codes).

## Shells

A shell is a terminado terminal on a server, addressed as `[SERVER:]SHELL`:

```sh
jhc shell new
jhc shell list
jhc shell send 1 -- python train.py
jhc shell peek 1 --follow
jhc shell attach 1
jhc shell kill 1
```

`send` writes a command and returns immediately; `peek` prints recent output; `attach` takes over your terminal interactively (detach with a double `Ctrl-\`).

`peek` shows only the terminal's bounded scrollback, so a long-running job's output can scroll past the ceiling and be lost. For long jobs, capture to a file and fetch it:

```sh
jhc shell send 1 -- 'cmd |& tee job.log'
jhc cp :job.log ./job.log
```

## File Transfer

```sh
jhc ls :results/
jhc cp :results/out.csv ./out.csv
jhc cp ./dataset.zip :data/dataset.zip
jhc cp -r ./dataset :data/dataset
jhc rm -r :scratch
```

`cp` needs exactly one remote side: it uploads (local to remote) or downloads (remote to local). Copying or deleting a directory requires `-r`.

## Addressing Servers And Shells

Most file and shell commands take an address that can name a server:

- Files use `[SERVER:]PATH`: `results/out.csv`, `:results/out.csv`, and `backup:results/out.csv`.
- Shells use `[SERVER:]SHELL`: `1`, `:1`, and `backup:1`.

With no server prefix (or an empty prefix like `:1`), the command targets your default server. In `jhc cp`, an argument is treated as local unless its first `:` comes before its first `/`, so `./weird:name.txt` and `dir/with:colon` stay local while `backup:runs` is remote.

## Multiple Hubs

A config can hold several named hub profiles. Add another with a `--name`:

```sh
jhc init --url https://other.example.edu --token TOKEN --name backup
```

Every command targets the `default_hub` unless you override it per-invocation:

```sh
jhc --hub backup status
jhc --hub backup start
```

See [Configuration](configuration.md#hub-profiles) for the file layout.
