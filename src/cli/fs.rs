use super::{CliError, Ctx};

pub async fn ls(_ctx: &Ctx, _path: &str) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}

pub async fn cp(_ctx: &Ctx, _src: &str, _dst: &str, _recursive: bool) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}

pub async fn rm(_ctx: &Ctx, _path: &str, _recursive: bool) -> Result<(), CliError> {
    Err(CliError::Usage(
        "implemented in a later task of this plan".to_string(),
    ))
}
