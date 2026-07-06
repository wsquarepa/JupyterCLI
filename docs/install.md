# Installation

## Install Script

```sh
curl -fsSL https://raw.githubusercontent.com/wsquarepa/JupyterCLI/master/install.sh | sh
```

The script supports Linux and macOS. It asks for a release channel:

- `stable`: latest stable release.
- `preview`: bleeding-edge build from `master`.
- `branch`: prerelease build for a development branch.

By default it installs to an existing `jhc` location if one is found, `/usr/local/bin` when run as root, or `~/.local/bin` otherwise.

Set `INSTALL_DIR` to choose a directory:

```sh
INSTALL_DIR="$HOME/bin" sh -c "$(curl -fsSL https://raw.githubusercontent.com/wsquarepa/JupyterCLI/master/install.sh)"
```

For private release access, set `GITHUB_TOKEN` or `GH_TOKEN` before running the script.

## Release Downloads

Prebuilt release assets are published for:

- Linux: `x86_64`, `aarch64`
- macOS: `x86_64`, `aarch64`

Download the matching `jhc-<target>` asset from [Releases](https://github.com/wsquarepa/JupyterCLI/releases), make it executable on Unix-like systems, and place it somewhere on your `PATH`.

## Build From Source

Requires the stable Rust toolchain from [rustup](https://rustup.rs/).

```sh
git clone https://github.com/wsquarepa/JupyterCLI.git
cd JupyterCLI
cargo build --release
```

The compiled binary is `target/release/jhc`. Copy it somewhere on your `PATH`.

## Updating

JupyterCLI has no built-in updater. Re-run the install script to update in place; when `jhc` is already on your `PATH`, the script detects it and overwrites that binary.

```sh
curl -fsSL https://raw.githubusercontent.com/wsquarepa/JupyterCLI/master/install.sh | sh
```

Pick the same channel you installed from. Switching channels may change application behavior or data expectations, so choose `preview` or a branch only when you know you want that build.
