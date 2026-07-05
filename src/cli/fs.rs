use crate::api::server::ServerClient;

use super::addr::{RemoteSide, TransferSide, parse_transfer_arg};
use super::shell::server_client_for;
use super::{CliError, Ctx};

pub async fn ls(ctx: &Ctx, path: &str) -> Result<(), CliError> {
    let remote = match parse_transfer_arg(path) {
        TransferSide::Remote(r) => r,
        TransferSide::Local(_) => {
            return Err(CliError::Usage(
                "ls takes a remote path: [SERVER:]PATH, e.g. :work or backup:data".to_string(),
            ));
        }
    };
    let (client, _) = server_client_for(ctx, remote.server.as_deref()).await?;
    let entries = client.list_dir(&remote.path).await?;
    println!("{:<30} {:<10} {:>12} LAST MODIFIED", "NAME", "TYPE", "SIZE");
    for entry in entries {
        let size = entry
            .size
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{:<30} {:<10} {:>12} {}",
            entry.name,
            entry.kind,
            size,
            entry.last_modified.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

pub async fn cp(ctx: &Ctx, src: &str, dst: &str, recursive: bool) -> Result<(), CliError> {
    match (parse_transfer_arg(src), parse_transfer_arg(dst)) {
        (TransferSide::Local(local), TransferSide::Remote(remote)) => {
            let (client, _) = server_client_for(ctx, remote.server.as_deref()).await?;
            upload_path(
                &client,
                std::path::Path::new(&local),
                &remote.path,
                recursive,
            )
            .await
        }
        (TransferSide::Remote(remote), TransferSide::Local(local)) => {
            let (client, _) = server_client_for(ctx, remote.server.as_deref()).await?;
            download_path(&client, &remote, std::path::Path::new(&local), recursive).await
        }
        _ => Err(CliError::Usage(
            "cp needs exactly one side remote: spell the remote side [SERVER:]PATH".to_string(),
        )),
    }
}

async fn upload_path(
    client: &ServerClient,
    local: &std::path::Path,
    remote: &str,
    recursive: bool,
) -> Result<(), CliError> {
    let meta = std::fs::metadata(local)?;
    if meta.is_dir() {
        if !recursive {
            return Err(CliError::Usage(format!(
                "{} is a directory; pass -r to copy it",
                local.display()
            )));
        }
        client.mkdir(remote).await?;
        for entry in std::fs::read_dir(local)? {
            let entry = entry?;
            let child_remote = format!("{}/{}", remote, entry.file_name().to_string_lossy());
            Box::pin(upload_path(client, &entry.path(), &child_remote, true)).await?;
        }
        return Ok(());
    }
    let bytes = std::fs::read(local)?;
    client.upload(remote, &bytes).await?;
    println!("{} -> :{remote}", local.display());
    Ok(())
}

async fn download_path(
    client: &ServerClient,
    remote: &RemoteSide,
    local: &std::path::Path,
    recursive: bool,
) -> Result<(), CliError> {
    // Try the file path first: it is the common case and needs only one request.
    // A directory target makes `download` fail (its content is not a string), so
    // on error we confirm via `list_dir` and recurse; a confirmed non-directory
    // re-raises the original download error instead of masking it.
    let download_err = match client.download(&remote.path).await {
        Ok(bytes) => {
            if let Some(parent) = local.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(local, bytes)?;
            println!(":{} -> {}", remote.path, local.display());
            return Ok(());
        }
        Err(e) => e,
    };

    let entries = client.list_dir(&remote.path).await?;
    let is_dir = entries.len() != 1 || entries[0].path != remote.path.trim_start_matches('/');
    if !is_dir {
        return Err(CliError::Api(download_err));
    }
    if !recursive {
        return Err(CliError::Usage(format!(
            ":{} is a directory; pass -r to copy it",
            remote.path
        )));
    }
    std::fs::create_dir_all(local)?;
    for entry in entries {
        let child_remote = RemoteSide {
            server: remote.server.clone(),
            path: entry.path.clone(),
        };
        Box::pin(download_path(
            client,
            &child_remote,
            &local.join(&entry.name),
            true,
        ))
        .await?;
    }
    Ok(())
}

pub async fn rm(ctx: &Ctx, path: &str, recursive: bool) -> Result<(), CliError> {
    let remote = match parse_transfer_arg(path) {
        TransferSide::Remote(r) => r,
        TransferSide::Local(_) => {
            return Err(CliError::Usage(
                "rm takes a remote path: [SERVER:]PATH".to_string(),
            ));
        }
    };
    let (client, _) = server_client_for(ctx, remote.server.as_deref()).await?;
    let entries = client.list_dir(&remote.path).await?;
    let is_dir = entries.len() != 1 || entries[0].path != remote.path.trim_start_matches('/');
    if is_dir && !recursive {
        return Err(CliError::Usage(format!(
            ":{} is a directory; pass -r to delete it",
            remote.path
        )));
    }
    if is_dir {
        for entry in entries {
            let child = RemoteSide {
                server: remote.server.clone(),
                path: entry.path.clone(),
            };
            if entry.kind == "directory" {
                Box::pin(rm_remote_dir(&client, &child)).await?;
            } else {
                client.delete_path(&entry.path).await?;
            }
        }
    }
    client.delete_path(&remote.path).await?;
    println!("deleted :{}", remote.path);
    Ok(())
}

async fn rm_remote_dir(client: &ServerClient, remote: &RemoteSide) -> Result<(), CliError> {
    let entries = client.list_dir(&remote.path).await?;
    for entry in entries {
        if entry.kind == "directory" {
            let child = RemoteSide {
                server: remote.server.clone(),
                path: entry.path.clone(),
            };
            Box::pin(rm_remote_dir(client, &child)).await?;
        } else {
            client.delete_path(&entry.path).await?;
        }
    }
    client.delete_path(&remote.path).await?;
    Ok(())
}
