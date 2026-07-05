use super::{CliError, Ctx, TokenCmd};

pub async fn run(_ctx: &Ctx, _cmd: TokenCmd) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}
