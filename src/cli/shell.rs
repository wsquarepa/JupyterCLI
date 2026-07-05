use super::{CliError, Ctx, ShellCmd};

pub async fn run(_ctx: &Ctx, _cmd: ShellCmd) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}

pub async fn exec_cmd(
    _ctx: &Ctx,
    _server: Option<&str>,
    _shell: Option<&str>,
    _command: &str,
) -> Result<i32, CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}
