use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ci") => ci(),
        _ => {
            eprintln!("usage: cargo xtask ci");
            ExitCode::from(2)
        }
    }
}

fn ci() -> ExitCode {
    let steps: [(&str, &[&str]); 3] = [
        ("fmt", &["fmt", "--all", "--check"]),
        (
            "clippy",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        ("test", &["test", "--workspace"]),
    ];
    for (name, step_args) in steps {
        println!("xtask ci: cargo {name}");
        match Command::new(env!("CARGO")).args(step_args).status() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                eprintln!("xtask ci: cargo {name} failed with {status}");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("xtask ci: cannot run cargo {name}: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    println!("xtask ci: all checks passed");
    ExitCode::SUCCESS
}
