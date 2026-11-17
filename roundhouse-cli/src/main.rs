//! The `cargo roundhouse` binary.

use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use roundhouse_build::ARTIFACT_DIR;

mod check;
mod diff;
mod explain;
mod init;
mod prepare;
mod verify;

pub(crate) fn error(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}

#[derive(Parser)]
#[command(
    name = "cargo-roundhouse",
    bin_name = "cargo roundhouse",
    version,
    about = "Build-time contract generation for roundhouse",
    long_about = "Build-time contract generation for roundhouse.\n\n\
                  The contract binary is built under `target/roundhouse`, so `cargo roundhouse` \
                  never writes a bin or otherwise touches the source tree."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create .roundhouse/, install the build script, and print the api! snippet
    Init {
        /// Do not create or update build.rs
        #[arg(long)]
        no_build_script: bool,
    },
    /// Run the contract harness and write contract.json + openapi.json
    Prepare {
        /// Directory that receives contract.json and openapi.json
        #[arg(long, default_value = ARTIFACT_DIR)]
        out: PathBuf,
    },
    /// Verify the committed contract artifacts are current
    Check,
    /// Show the documented responses for one operation
    Explain {
        /// Operation id, path, or `METHOD /path`
        operation: String,
    },
    /// Run tests with response verification against the committed contract
    Verify {
        /// Arguments passed to `cargo test` after `--`
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Compare the committed contract with a previous version
    Diff {
        /// Compare against this contract.json instead of `git HEAD`
        #[arg(long)]
        against: Option<PathBuf>,
        /// Exit non-zero when a breaking change is found
        #[arg(long)]
        check: bool,
    },
}

fn main() {
    let cli = Cli::parse_from(normalized_args());
    if let Err(error) = run(cli) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Commands::Init { no_build_script } => init::init(no_build_script),
        Commands::Prepare { out } => prepare::prepare(out),
        Commands::Check => check::check(),
        Commands::Explain { operation } => explain::explain(&operation),
        Commands::Verify { args } => verify::verify(args),
        Commands::Diff { against, check } => diff::diff(against, check),
    }
}

/// Cargo may invoke the subcommand as `cargo-roundhouse roundhouse …` depending on the
/// installation; accept both `cargo roundhouse init` and `cargo-roundhouse roundhouse init`.
fn normalized_args() -> Vec<OsString> {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args.get(1).and_then(|arg| arg.to_str()) == Some("roundhouse") {
        args.remove(1);
    }
    args
}
