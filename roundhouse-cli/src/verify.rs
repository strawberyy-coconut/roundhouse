//! `cargo roundhouse verify`: run tests with response verification enabled.

use std::error::Error;
use std::ffi::OsString;
use std::process::Command;

use roundhouse_build::ARTIFACT_DIR;

use crate::error;

pub fn verify(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    if !cwd.join(ARTIFACT_DIR).join("contract.json").exists() {
        return Err(error(
            "no committed contract found; run `cargo roundhouse prepare` first",
        ));
    }

    let mut command = Command::new(cargo());
    command.arg("test");
    if !args.is_empty() {
        command.arg("--");
        command.args(args);
    }
    command.env("ROUNDHOUSE_VERIFY", "1");

    let status = command.status()?;
    if !status.success() {
        return Err(error(
            "response verification failed; a response did not match .roundhouse/contract.json",
        ));
    }
    Ok(())
}

fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}
