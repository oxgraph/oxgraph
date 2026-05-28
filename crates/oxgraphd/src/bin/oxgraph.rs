//! `oxgraph` command-line entry point.

use std::process::ExitCode;

/// Runs the CLI process.
fn main() -> ExitCode {
    match oxgraphd::run_cli(std::env::args().skip(1)) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
