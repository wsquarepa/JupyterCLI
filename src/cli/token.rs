use super::{CliError, Ctx, TokenCmd};

pub async fn run(ctx: &Ctx, cmd: TokenCmd) -> Result<(), CliError> {
    let user = ctx.client.whoami().await?;
    match cmd {
        TokenCmd::List => {
            let tokens = ctx.client.tokens(&user.name).await?;
            if tokens.is_empty() {
                println!("no API tokens");
                return Ok(());
            }
            println!("{:<12} {:<24} {:<22} EXPIRES", "ID", "NOTE", "CREATED");
            for token in tokens {
                println!(
                    "{:<12} {:<24} {:<22} {}",
                    token.id,
                    token.note.as_deref().unwrap_or("-"),
                    token.created.as_deref().unwrap_or("-"),
                    token.expires_at.as_deref().unwrap_or("never")
                );
            }
            Ok(())
        }
        TokenCmd::Create { note } => {
            let new = ctx.client.create_token(&user.name, &note).await?;
            println!("token id: {}", new.id);
            println!("{}", new.token);
            println!("store it now; it will not be shown again");
            Ok(())
        }
        TokenCmd::Revoke { id } => {
            ctx.client.revoke_token(&user.name, &id).await?;
            println!("revoked token {id}");
            Ok(())
        }
    }
}
