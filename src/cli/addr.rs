#[derive(Debug)]
pub struct RemoteSide {
    pub server: Option<String>,
    pub path: String,
}

#[derive(Debug)]
pub enum TransferSide {
    Local(String),
    Remote(RemoteSide),
}

pub fn parse_shell_ref(input: &str) -> (Option<String>, String) {
    match input.split_once(':') {
        Some(("", shell)) => (None, shell.to_string()),
        Some((server, shell)) => (Some(server.to_string()), shell.to_string()),
        None => (None, input.to_string()),
    }
}

pub fn parse_transfer_arg(input: &str) -> TransferSide {
    let colon = input.find(':');
    let slash = input.find('/');
    match colon {
        Some(c) if slash.is_none_or(|s| c < s) => {
            let (server, path) = input.split_at(c);
            TransferSide::Remote(RemoteSide {
                server: (!server.is_empty()).then(|| server.to_string()),
                path: path[1..].to_string(),
            })
        }
        _ => TransferSide::Local(input.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_refs() {
        assert_eq!(
            parse_shell_ref("backup:2"),
            (Some("backup".to_string()), "2".to_string())
        );
        assert_eq!(parse_shell_ref("2"), (None, "2".to_string()));
        assert_eq!(parse_shell_ref(":2"), (None, "2".to_string()));
    }

    #[test]
    fn transfer_scp_rule() {
        match parse_transfer_arg("backup:results/out.csv") {
            TransferSide::Remote(r) => {
                assert_eq!(r.server.as_deref(), Some("backup"));
                assert_eq!(r.path, "results/out.csv");
            }
            other => panic!("{other:?}"),
        }
        match parse_transfer_arg(":out.csv") {
            TransferSide::Remote(r) => {
                assert_eq!(r.server, None);
                assert_eq!(r.path, "out.csv");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse_transfer_arg("plain.txt"),
            TransferSide::Local(_)
        ));
        assert!(matches!(
            parse_transfer_arg("./weird:name.txt"),
            TransferSide::Local(_)
        ));
        assert!(matches!(
            parse_transfer_arg("dir/with:colon"),
            TransferSide::Local(_)
        ));
    }
}
