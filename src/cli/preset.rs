use super::{CliError, Ctx, PresetCmd};

pub async fn run(_ctx: &Ctx, _cmd: PresetCmd) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}
