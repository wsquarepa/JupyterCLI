pub mod addr;
pub mod fs;
pub mod init;
pub mod preset;
pub mod server;
pub mod shell;
pub mod token;

use clap::{Parser, Subcommand};

use crate::api::HubClient;
use crate::api::error::ApiError;
use crate::config::{self, ConfigError, HubConfig, JsonMap};

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

#[derive(Parser)]
#[command(
    name = "jhc",
    version,
    about = "JupyterCLI: manage JupyterHub servers and shells from the terminal",
    after_help = "Run jhc with no arguments to open the interactive interface."
)]
pub struct Cli {
    /// Hub profile from the config to use for this invocation
    #[arg(long, global = true)]
    pub hub: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create or update the JupyterCLI configuration
    Init {
        /// Hub base URL, e.g. https://jupyter.example.edu
        #[arg(long)]
        url: Option<String>,
        /// API token for the hub
        #[arg(long)]
        token: Option<String>,
        /// Name for this hub profile
        #[arg(long, default_value = "default")]
        name: String,
    },
    /// Show the authenticated user and all servers
    Status,
    /// Start a server
    Start {
        /// Named server to start; omit for the default server
        server: Option<String>,
        /// Spawn preset from the config
        #[arg(long)]
        preset: Option<String>,
        /// Spawn option key=value (string) or key:=JSON (typed); repeatable
        #[arg(short = 'o', long = "option")]
        options: Vec<String>,
        /// Return immediately instead of streaming spawn progress
        #[arg(long)]
        no_wait: bool,
    },
    /// Stop a server
    Stop {
        /// Named server to stop; omit for the default server
        server: Option<String>,
    },
    /// Manage spawn presets
    #[command(subcommand)]
    Preset(PresetCmd),
    /// Manage shells on a server
    #[command(subcommand)]
    Shell(ShellCmd),
    /// Run a command on a server and exit with its status
    Exec {
        /// Server to run on; omit for the default server
        server: Option<String>,
        /// Existing shell to reuse instead of an ephemeral one, as [SERVER:]SHELL
        #[arg(long)]
        shell: Option<String>,
        /// Command and arguments to run
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// List remote files
    Ls {
        /// Remote path as [SERVER:]PATH
        path: String,
    },
    /// Copy files between the local machine and a server
    Cp {
        /// Source: local path or [SERVER:]PATH
        src: String,
        /// Destination: local path or [SERVER:]PATH
        dst: String,
        /// Copy directories recursively
        #[arg(short, long)]
        recursive: bool,
    },
    /// Delete a remote file or directory
    Rm {
        /// Remote path as [SERVER:]PATH
        path: String,
        /// Delete directories recursively
        #[arg(short, long)]
        recursive: bool,
    },
    /// Manage hub API tokens
    #[command(subcommand)]
    Token(TokenCmd),
}

#[derive(Subcommand)]
pub enum PresetCmd {
    /// Save a running server's options as a preset
    Import {
        /// Server whose options to capture; omit for the default server
        server: Option<String>,
        /// Preset name to save under
        #[arg(long = "as", default_value = "imported")]
        name: String,
    },
    /// List configured presets
    List,
}

#[derive(Subcommand)]
pub enum ShellCmd {
    /// Create a shell on a server
    New {
        /// Server to create the shell on; omit for the default server
        server: Option<String>,
    },
    /// List shells on a server
    List {
        /// Server to list; omit for the default server
        server: Option<String>,
    },
    /// Write a command line to a shell and return immediately
    Send {
        /// Shell as [SERVER:]SHELL
        shell: String,
        /// Text to send; a newline is appended
        #[arg(trailing_var_arg = true, required = true)]
        text: Vec<String>,
    },
    /// Print a shell's recent output
    #[command(
        after_help = "peek shows only the terminal's bounded scrollback, so output from a \
long-running job can scroll past the ceiling and be lost. For long jobs, capture to a file and \
fetch it instead:\n  jhc shell send SHELL -- 'cmd |& tee job.log'\n  jhc cp :job.log ./job.log"
    )]
    Peek {
        /// Shell as [SERVER:]SHELL
        shell: String,
        /// Keep streaming live output until interrupted
        #[arg(long)]
        follow: bool,
        /// Emit verbatim bytes instead of stripped text
        #[arg(long)]
        raw: bool,
    },
    /// Destroy a shell
    Kill {
        /// Shell as [SERVER:]SHELL
        shell: String,
    },
    /// Attach interactively; press Ctrl-\ twice to detach
    Attach {
        /// Shell as [SERVER:]SHELL
        shell: String,
    },
}

#[derive(Subcommand)]
pub enum TokenCmd {
    /// List your API tokens
    List,
    /// Create a token and print it
    Create {
        /// Note recorded with the token
        #[arg(long, default_value = "created by JupyterCLI")]
        note: String,
    },
    /// Revoke a token by id
    Revoke {
        /// Token id from token list
        id: String,
    },
}

pub struct Ctx {
    pub hub_name: String,
    pub hub: HubConfig,
    pub client: HubClient,
}

impl Ctx {
    pub fn load(hub_flag: Option<&str>) -> Result<Self, CliError> {
        let cfg = config::load()?;
        if let Ok(path) = config::path()
            && config::permissions_are_loose(&path)
        {
            eprintln!(
                "warning: {} is readable by other users; run: chmod 600 {}",
                path.display(),
                path.display()
            );
        }
        let (name, hub) = cfg.resolve_hub(hub_flag)?;
        let client = HubClient::new(&hub.url, &hub.effective_token())?;
        Ok(Self {
            hub_name: name.to_string(),
            hub: hub.clone(),
            client,
        })
    }
}

pub fn parse_option_kv(raw: &str) -> Result<(String, serde_json::Value), CliError> {
    if let Some((key, json)) = raw.split_once(":=") {
        let value = serde_json::from_str(json)
            .map_err(|e| CliError::Usage(format!("invalid JSON in option '{raw}': {e}")))?;
        return Ok((key.to_string(), value));
    }
    match raw.split_once('=') {
        Some((key, value)) => Ok((
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        )),
        None => Err(CliError::Usage(format!(
            "option '{raw}' must be key=value or key:=JSON"
        ))),
    }
}

pub fn options_from_flags(flags: &[String]) -> Result<JsonMap, CliError> {
    let mut map = JsonMap::new();
    for raw in flags {
        let (key, value) = parse_option_kv(raw)?;
        map.insert(key, value);
    }
    Ok(map)
}

pub fn main() -> std::process::ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            // exec reserves every non-125 exit for the remote command's status, so a clap
            // usage error under exec must surface as 125, not clap's default 2. Heuristic:
            // the first non-flag argument names the subcommand. `use_stderr()` is true only
            // for real errors, so --help/--version (exit 0, stdout) still take err.exit().
            let targets_exec = std::env::args()
                .skip(1)
                .find(|arg| !arg.starts_with('-'))
                .as_deref()
                == Some("exec");
            if targets_exec && err.use_stderr() {
                eprint!("{err}");
                return std::process::ExitCode::from(crate::shellops::JHC_FAILURE_EXIT as u8);
            }
            err.exit();
        }
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: cannot start async runtime: {e}");
            return std::process::ExitCode::from(1);
        }
    };
    let is_exec = matches!(cli.command, Some(Command::Exec { .. }));
    match runtime.block_on(dispatch(cli)) {
        Ok(code) => code,
        Err(CliError::Config(ConfigError::NotFound(_))) => {
            eprintln!("{}", init::NO_CONFIG_GUIDANCE);
            exit_code_for_failure(is_exec)
        }
        Err(e) => {
            eprintln!("error: {e}");
            let mut source = std::error::Error::source(&e);
            while let Some(inner) = source {
                eprintln!("  caused by: {inner}");
                source = inner.source();
            }
            exit_code_for_failure(is_exec)
        }
    }
}

fn exit_code_for_failure(is_exec: bool) -> std::process::ExitCode {
    if is_exec {
        std::process::ExitCode::from(crate::shellops::JHC_FAILURE_EXIT as u8)
    } else {
        std::process::ExitCode::from(1)
    }
}

async fn dispatch(cli: Cli) -> Result<std::process::ExitCode, CliError> {
    let ok = std::process::ExitCode::SUCCESS;
    match cli.command {
        None => {
            println!(
                "The JupyterCLI interactive interface arrives in a later milestone.\nRun jhc --help for the available commands."
            );
            Ok(ok)
        }
        Some(Command::Init { url, token, name }) => {
            init::run(url, token, name).await?;
            Ok(ok)
        }
        Some(Command::Status) => {
            server::status(&Ctx::load(cli.hub.as_deref())?).await?;
            Ok(ok)
        }
        Some(Command::Start {
            server,
            preset,
            options,
            no_wait,
        }) => {
            server::start(
                &Ctx::load(cli.hub.as_deref())?,
                server.as_deref(),
                preset.as_deref(),
                &options,
                no_wait,
            )
            .await?;
            Ok(ok)
        }
        Some(Command::Stop { server }) => {
            server::stop(&Ctx::load(cli.hub.as_deref())?, server.as_deref()).await?;
            Ok(ok)
        }
        Some(Command::Preset(cmd)) => {
            preset::run(&Ctx::load(cli.hub.as_deref())?, cmd).await?;
            Ok(ok)
        }
        Some(Command::Shell(cmd)) => {
            shell::run(&Ctx::load(cli.hub.as_deref())?, cmd).await?;
            Ok(ok)
        }
        Some(Command::Exec {
            server,
            shell,
            command,
        }) => {
            if command.is_empty() {
                return Err(CliError::Usage(
                    "exec requires a command to run".to_string(),
                ));
            }
            let code = shell::exec_cmd(
                &Ctx::load(cli.hub.as_deref())?,
                server.as_deref(),
                shell.as_deref(),
                &command.join(" "),
            )
            .await?;
            Ok(std::process::ExitCode::from(code as u8))
        }
        Some(Command::Ls { path }) => {
            fs::ls(&Ctx::load(cli.hub.as_deref())?, &path).await?;
            Ok(ok)
        }
        Some(Command::Cp {
            src,
            dst,
            recursive,
        }) => {
            fs::cp(&Ctx::load(cli.hub.as_deref())?, &src, &dst, recursive).await?;
            Ok(ok)
        }
        Some(Command::Rm { path, recursive }) => {
            fs::rm(&Ctx::load(cli.hub.as_deref())?, &path, recursive).await?;
            Ok(ok)
        }
        Some(Command::Token(cmd)) => {
            token::run(&Ctx::load(cli.hub.as_deref())?, cmd).await?;
            Ok(ok)
        }
    }
}
