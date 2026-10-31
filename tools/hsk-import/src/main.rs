//! `hsk-import` command line; see `hsk_import::cli::USAGE` and
//! crates/vocab-core/data/README.md for the regeneration steps.

use std::process::ExitCode;

use hsk_import::{cli, Error};

fn main() -> ExitCode {
    let result = cli::parse_args(std::env::args().skip(1)).and_then(cli::run);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage(message)) => {
            eprintln!("hsk-import: {message}\n\n{}", cli::USAGE);
            ExitCode::from(2)
        }
        Err(err) => {
            eprintln!("hsk-import: {err}");
            ExitCode::FAILURE
        }
    }
}
