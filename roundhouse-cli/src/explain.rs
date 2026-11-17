//! `cargo roundhouse explain`: print the documented responses for one operation.

use std::error::Error;
use std::fs;

use roundhouse_build::ARTIFACT_DIR;
use serde_json::Value;

use crate::error;

pub fn explain(operation: &str) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let path = cwd.join(ARTIFACT_DIR).join("contract.json");
    let contract: Value = serde_json::from_str(&fs::read_to_string(&path).map_err(|_| {
        error("no committed contract found; run `cargo roundhouse prepare` first")
    })?)?;
    let operations = contract["operations"]
        .as_array()
        .ok_or_else(|| error("contract has no operations"))?;

    let wanted = operation.trim();
    let wanted_method_path = wanted.split_once(' ');
    let found = operations.iter().find(|candidate| {
        let id = candidate["operation_id"].as_str().unwrap_or_default();
        let path = candidate["path"].as_str().unwrap_or_default();
        let method = candidate["method"].as_str().unwrap_or_default();
        id == wanted
            || path == wanted
            || wanted_method_path.is_some_and(|(wanted_method, wanted_path)| {
                method.eq_ignore_ascii_case(wanted_method) && path == wanted_path
            })
    });

    let Some(operation) = found else {
        let ids: Vec<&str> = operations
            .iter()
            .filter_map(|operation| operation["operation_id"].as_str())
            .collect();
        return Err(error(format!(
            "no operation matched `{wanted}`; known operations: {}",
            ids.join(", ")
        )));
    };

    println!(
        "{} {} ({})",
        operation["method"].as_str().unwrap_or("?"),
        operation["path"].as_str().unwrap_or("?"),
        operation["operation_id"].as_str().unwrap_or("?")
    );
    if let (Some(file), Some(line)) = (
        operation["source"]["file"].as_str(),
        operation["source"]["line"].as_u64(),
    ) {
        println!("  at {file}:{line}");
    }
    let Some(responses) = operation["responses"].as_array() else {
        println!("  no responses documented");
        return Ok(());
    };
    for response in responses {
        let status = response["status"].as_u64().map_or_else(
            || "default".to_string(),
            |status| {
                if status == 0 {
                    "default".to_string()
                } else {
                    status.to_string()
                }
            },
        );
        let content_type = response["content"]
            .as_object()
            .and_then(|content| content.keys().next())
            .map_or("-", String::as_str);
        let schema_value = &response["content"][content_type]["schema"];
        let schema = schema_value["$ref"]
            .as_str()
            .or_else(|| schema_value.is_object().then_some("(inline)"))
            .unwrap_or("-");
        let description = response["description"].as_str().unwrap_or("");
        println!("  {status:<7} {content_type:<28} {schema:<36} {description}");
    }
    Ok(())
}
