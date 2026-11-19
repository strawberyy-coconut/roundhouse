//! Refresh `roundhouse-schema/data/status-codes.json` from the IANA registry.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use roundhouse_status_codes::{SOURCE_URL, USER_AGENT, normalize, render, snapshot};

const USAGE: &str = "\
usage: update-status-codes [--input <csv>] [--output <json>] [--check]

  --input <csv>    read a local CSV instead of fetching IANA
  --output <json>  write the snapshot here
                   (default: roundhouse-schema/data/status-codes.json)
  --check          compare the generated snapshot with the committed file and exit 1 if stale
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut input = None;
    let mut output = default_output();
    let mut check = false;

    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--input" => {
                input = Some(PathBuf::from(
                    arguments.next().context("`--input` needs a path")?,
                ));
            }
            "--output" => {
                output = PathBuf::from(arguments.next().context("`--output` needs a path")?);
            }
            "--check" => check = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown argument `{other}`\n\n{USAGE}"),
        }
    }

    let csv = match input {
        Some(path) => {
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?
        }
        None => fetch(SOURCE_URL)?,
    };

    let codes = normalize(&csv).context("normalizing the IANA registry")?;
    let rendered = render(&snapshot(codes))?;

    if check {
        let committed =
            fs::read_to_string(&output).with_context(|| format!("reading {}", output.display()))?;
        if committed == rendered {
            println!("{} is up to date", output.display());
            return Ok(());
        }
        bail!(
            "{} is stale; run `cargo run -p roundhouse-status-codes --bin update-status-codes`",
            output.display()
        );
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&output, rendered).with_context(|| format!("writing {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

fn fetch(url: &str) -> Result<String> {
    let mut response = ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .with_context(|| format!("fetching {url}"))?;
    response
        .body_mut()
        .read_to_string()
        .context("reading the IANA response body")
}

fn default_output() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("roundhouse-schema")
        .join("data")
        .join("status-codes.json")
}
