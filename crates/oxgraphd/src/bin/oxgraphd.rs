//! `oxgraphd` HTTP server entry point.

use std::process::ExitCode;

/// Runs the daemon process.
fn main() -> ExitCode {
    match oxgraphd::run_daemon_cli(std::env::args().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
