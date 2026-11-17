//! `cargo roundhouse prepare`: generate the contract and report what was written.

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

pub fn prepare(out: PathBuf) -> Result<(), Box<dyn Error>> {
    roundhouse_build::prepare_to(out.clone())?;

    let contract: Value = serde_json::from_str(&fs::read_to_string(out.join("contract.json"))?)?;
    let operations = contract["operations"]
        .as_array()
        .map_or(0, |operations| operations.len());
    println!(
        "wrote {} and {} ({operations} operations)",
        out.join("contract.json").display(),
        out.join("openapi.json").display()
    );
    Ok(())
}
