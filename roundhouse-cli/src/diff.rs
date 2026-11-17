//! `cargo roundhouse diff`: compare the committed contract with a previous one.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use roundhouse_build::ARTIFACT_DIR;
use serde_json::Value;

use crate::error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Breaking,
    Compatible,
}

#[derive(Debug, Clone)]
struct Change {
    severity: Severity,
    route: Option<(String, String)>,
    detail: String,
}

pub fn diff(against: Option<PathBuf>, check: bool) -> Result<(), Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let current_path = cwd.join(ARTIFACT_DIR).join("contract.json");
    if !current_path.exists() {
        return Err(error(
            "no committed contract found; run `cargo roundhouse prepare` first",
        ));
    }
    let current: Value = serde_json::from_str(&fs::read_to_string(&current_path)?)?;

    let previous: Value = if let Some(path) = against {
        serde_json::from_str(
            &fs::read_to_string(path)
                .map_err(|_| error("could not read the `--against` contract"))?,
        )?
    } else {
        serde_json::from_str(&git_show_contract(&cwd, &current_path)?)?
    };

    let changes = diff_contracts(&previous, &current);
    let breaking = changes
        .iter()
        .filter(|change| change.severity == Severity::Breaking)
        .count();
    let compatible = changes.len() - breaking;

    let mut grouped: std::collections::BTreeMap<Option<(String, String)>, Vec<&Change>> =
        std::collections::BTreeMap::new();
    for change in &changes {
        grouped
            .entry(change.route.clone())
            .or_default()
            .push(change);
    }
    for (route, route_changes) in grouped {
        if let Some((method, path)) = route {
            println!("{method} {path}");
            for change in route_changes {
                println!("  {:<10} {}", label(change.severity), change.detail);
            }
        } else {
            for change in route_changes {
                println!("{:<10} {}", label(change.severity), change.detail);
            }
        }
    }

    println!("{breaking} breaking, {compatible} compatible");
    if check && breaking > 0 {
        return Err(error(format!(
            "{breaking} breaking contract change(s); review them or omit `--check`"
        )));
    }
    Ok(())
}

fn label(severity: Severity) -> &'static str {
    match severity {
        Severity::Breaking => "breaking",
        Severity::Compatible => "compatible",
    }
}

fn git_show_contract(cwd: &Path, contract_path: &Path) -> Result<String, Box<dyn Error>> {
    let root = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()?;
    if !root.status.success() {
        return Err(error(
            "not in a git repository; pass `--against <contract.json>`",
        ));
    }
    let root = String::from_utf8(root.stdout)?;
    let root = Path::new(root.trim());
    let path = contract_path
        .canonicalize()?
        .strip_prefix(root)
        .map_err(|_| error("the contract is outside the git repository"))?
        .to_string_lossy()
        .replace('\\', "/");
    let output = Command::new("git")
        .args(["show", &format!("HEAD:{path}")])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(error(
            "no contract in git HEAD; commit one or pass `--against <contract.json>`",
        ));
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn diff_contracts(previous: &Value, current: &Value) -> Vec<Change> {
    use std::collections::{BTreeMap, BTreeSet};

    type Route = (String, String);
    fn routes(contract: &Value) -> BTreeMap<Route, &Value> {
        contract["operations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|operation| {
                Some((
                    (
                        operation["method"].as_str()?.to_string(),
                        operation["path"].as_str()?.to_string(),
                    ),
                    operation,
                ))
            })
            .collect()
    }
    fn responses(operation: &Value) -> BTreeMap<String, BTreeSet<String>> {
        operation["responses"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|response| {
                let status = response["status"].as_u64()?;
                let content = response["content"]
                    .as_object()
                    .map(|content| content.keys().cloned().collect())
                    .unwrap_or_default();
                Some((status.to_string(), content))
            })
            .collect()
    }
    fn request_body(operation: &Value) -> Option<(String, String)> {
        let body = &operation["request"]["body"];
        if body.is_null() {
            return None;
        }
        let content_type = body["content_type"].as_str()?.to_string();
        let schema = body["schema"]["$ref"]
            .as_str()
            .unwrap_or("<inline>")
            .to_string();
        Some((content_type, schema))
    }
    fn security(operation: &Value) -> Vec<String> {
        operation["security"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|scheme| scheme.as_str().map(str::to_string))
            .collect()
    }

    let previous_routes = routes(previous);
    let current_routes = routes(current);
    let mut changes = Vec::new();

    for (route, previous_operation) in &previous_routes {
        let Some(current_operation) = current_routes.get(route) else {
            changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "operation removed".to_string(),
            });
            continue;
        };

        if previous_operation["operation_id"] != current_operation["operation_id"] {
            changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "operation id changed".to_string(),
            });
        }

        match (
            request_body(previous_operation),
            request_body(current_operation),
        ) {
            (None, Some(_)) => changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "request body added".to_string(),
            }),
            (Some(_), None) => changes.push(Change {
                severity: Severity::Compatible,
                route: Some(route.clone()),
                detail: "request body removed".to_string(),
            }),
            (Some(previous), Some(current)) if previous != current => changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "request body changed".to_string(),
            }),
            _ => {}
        }

        if security(previous_operation) != security(current_operation) {
            changes.push(Change {
                severity: Severity::Breaking,
                route: Some(route.clone()),
                detail: "security changed".to_string(),
            });
        }

        let previous_responses = responses(previous_operation);
        let current_responses = responses(current_operation);
        for (status, previous_content) in &previous_responses {
            let Some(current_content) = current_responses.get(status) else {
                changes.push(Change {
                    severity: Severity::Breaking,
                    route: Some(route.clone()),
                    detail: format!("response {status} removed"),
                });
                continue;
            };
            for content_type in previous_content.difference(current_content) {
                changes.push(Change {
                    severity: Severity::Breaking,
                    route: Some(route.clone()),
                    detail: format!("response {status} no longer documents {content_type}"),
                });
            }
            for content_type in current_content.difference(previous_content) {
                changes.push(Change {
                    severity: Severity::Compatible,
                    route: Some(route.clone()),
                    detail: format!("response {status} adds {content_type}"),
                });
            }
        }
        for status in current_responses.keys() {
            if !previous_responses.contains_key(status) {
                changes.push(Change {
                    severity: Severity::Compatible,
                    route: Some(route.clone()),
                    detail: format!("response {status} added"),
                });
            }
        }
    }

    for route in current_routes.keys() {
        if !previous_routes.contains_key(route) {
            changes.push(Change {
                severity: Severity::Compatible,
                route: Some(route.clone()),
                detail: "operation added".to_string(),
            });
        }
    }

    changes.sort_by(|left, right| {
        left.route
            .cmp(&right.route)
            .then_with(|| left.detail.cmp(&right.detail))
    });
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_reports_breaking_and_compatible_changes() {
        let previous = serde_json::json!({
            "operations": [{
                "operation_id": "show",
                "method": "GET",
                "path": "/notes/{id}",
                "security": [],
                "request": { "body": null },
                "responses": [
                    { "status": 200, "content": { "application/json": {} } },
                    { "status": 404, "content": { "application/problem+json": {} } }
                ]
            }]
        });
        let current = serde_json::json!({
            "operations": [{
                "operation_id": "show",
                "method": "GET",
                "path": "/notes/{id}",
                "security": ["Bearer"],
                "request": { "body": { "content_type": "application/json", "schema": {} } },
                "responses": [
                    { "status": 200, "content": { "application/json": {} } },
                    { "status": 500, "content": { "application/problem+json": {} } }
                ]
            }]
        });

        let changes = diff_contracts(&previous, &current);
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Breaking && change.detail == "security changed"
        }));
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Breaking && change.detail == "request body added"
        }));
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Breaking && change.detail == "response 404 removed"
        }));
        assert!(changes.iter().any(|change| {
            change.severity == Severity::Compatible && change.detail == "response 500 added"
        }));
    }

    #[test]
    fn diff_reports_removed_operations_as_breaking() {
        let previous = serde_json::json!({
            "operations": [{
                "operation_id": "old",
                "method": "GET",
                "path": "/old",
                "security": [],
                "request": { "body": null },
                "responses": []
            }]
        });
        let current = serde_json::json!({ "operations": [] });
        let changes = diff_contracts(&previous, &current);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].severity, Severity::Breaking);
        assert_eq!(changes[0].detail, "operation removed");
    }
}
