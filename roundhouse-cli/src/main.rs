//! `roundhouse openapi` — dump a linked app's document, for CI and golden files.
//!
//! ```sh
//! roundhouse openapi -p notes-example
//! roundhouse openapi -p notes-example --format 3.0
//! ```

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::process::Command;

/// Dump a linked app's OpenAPI document or resolved route table.
#[derive(Parser)]
#[command(name = "roundhouse", version, about)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    /// Print the document App::serve would have served.
    Openapi {
        /// Package to run.
        #[arg(short, long)]
        package: String,

        /// OpenAPI version to emit.
        #[arg(long, value_enum, default_value_t = Format::V31)]
        format: Format,

        /// Write to a file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Print the resolved route table with each declaration's source.
    Routes {
        /// Package to run.
        #[arg(short, long)]
        package: String,

        /// Write to a file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum Format {
    #[default]
    #[value(name = "3.1", alias = "3.1.0")]
    V31,
    #[value(name = "3.0", alias = "3.0.3")]
    V30,
}

impl Format {
    fn flag(self) -> &'static str {
        match self {
            Self::V31 => "--openapi",
            Self::V30 => "--openapi-30",
        }
    }
}

fn main() {
    let (package, flag, output) = match Cli::parse().command {
        CommandKind::Openapi {
            package,
            format,
            output,
        } => (package, format.flag(), output),
        CommandKind::Routes { package, output } => (package, "--routes", output),
    };

    let mut command = Command::new("cargo");
    command.args(["run", "-q", "-p", &package, "--", flag]);
    if let Some(output) = output {
        command.arg("--output").arg(output);
    }

    let status = command.status().expect("cargo is available");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}
