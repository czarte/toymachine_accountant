//! Command-line entry point: `toymachine <transactions.csv>`.
//!
//! Streams the input file through the `data_stream` and `accountant` actors
//! and prints the resulting client balances as CSV on stdout.

use std::process::ExitCode;

mod process;
mod utils;

const USAGE: &str = "Usage: cargo run -- <transactions.csv> > <output.csv>";

#[tokio::main]
async fn main() -> ExitCode {
    let Some(path) = utils::input_path(std::env::args_os()) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    match process::process(path).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
