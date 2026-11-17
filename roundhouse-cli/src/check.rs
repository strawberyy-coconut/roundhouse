//! `cargo roundhouse check`: regenerate and compare against the committed artifacts.

use std::error::Error;
use std::fs;
use std::path::Path;

use roundhouse_build::ARTIFACT_DIR;
use serde_json::Value;

use crate::error;

pub fn check() -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let committed = cwd.join(ARTIFACT_DIR);
    if !committed.join("contract.json").exists() || !committed.join("openapi.json").exists() {
        return Err(error(
            "no committed contract found; run `cargo roundhouse prepare` and commit .roundhouse/",
        ));
    }

    let temporary = cwd
        .join("target")
        .join(format!("roundhouse-check-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    fs::create_dir_all(&temporary)?;

    let result = roundhouse_build::prepare_to(temporary.clone())
        .and_then(|()| compare_artifacts(&committed, &temporary));
    let _ = fs::remove_dir_all(&temporary);
    result?;

    println!("roundhouse contract is current");
    Ok(())
}

fn compare_artifacts(committed: &Path, generated: &Path) -> Result<(), Box<dyn Error>> {
    for name in ["contract.json", "openapi.json"] {
        let committed: Value = serde_json::from_str(&fs::read_to_string(committed.join(name))?)?;
        let generated: Value = serde_json::from_str(&fs::read_to_string(generated.join(name))?)?;
        if committed != generated {
            return Err(error(format!(
                "{name} is stale; run `cargo roundhouse prepare` and commit the result"
            )));
        }
    }
    Ok(())
}
