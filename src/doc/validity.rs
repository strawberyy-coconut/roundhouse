//! Structural validation of the emitted document.
//!
//! This is the invariant the project exists for, checked on the final artifact rather than on the
//! declarations: every template capture has exactly one required path parameter, every path
//! parameter is used, operation ids are unique, every security requirement names a declared
//! scheme, and every operation has at least one response.

use utoipa::openapi::OpenApi;

const OPERATIONS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Check the document's internal consistency. Called on every build.
pub fn validate_document(document: &OpenApi) -> Result<(), String> {
    let value = serde_json::to_value(document).map_err(|error| error.to_string())?;

    let schemes: Vec<&str> = value
        .pointer("/components/securitySchemes")
        .and_then(|value| value.as_object())
        .map(|object| object.keys().map(String::as_str).collect())
        .unwrap_or_default();

    let Some(paths) = value.get("paths").and_then(|value| value.as_object()) else {
        return Ok(());
    };

    let mut operation_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (path, item) in paths {
        let captures = crate::prefix::captures(path);
        for method in OPERATIONS {
            let Some(operation) = item.get(method) else {
                continue;
            };

            if let Some(id) = operation.get("operationId").and_then(|id| id.as_str())
                && !operation_ids.insert(id.to_string())
            {
                return Err(format!("duplicate operationId `{id}`"));
            }

            let parameters = operation
                .get("parameters")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let mut path_parameters: Vec<&str> = Vec::new();
            for parameter in &parameters {
                if parameter.get("in").and_then(|value| value.as_str()) != Some("path") {
                    continue;
                }
                let Some(name) = parameter.get("name").and_then(|value| value.as_str()) else {
                    return Err(format!("{method} {path}: path parameter without a name"));
                };
                if parameter.get("required").and_then(|value| value.as_bool()) != Some(true) {
                    return Err(format!(
                        "{method} {path}: path parameter `{name}` must be required"
                    ));
                }
                if !captures.iter().any(|capture| capture == name) {
                    return Err(format!(
                        "{method} {path}: path parameter `{name}` is not a template capture"
                    ));
                }
                if path_parameters.contains(&name) {
                    return Err(format!(
                        "{method} {path}: path parameter `{name}` is declared twice"
                    ));
                }
                path_parameters.push(name);
            }
            for capture in &captures {
                if !path_parameters.contains(&capture.as_str()) {
                    return Err(format!(
                        "{method} {path}: capture `{capture}` has no path parameter"
                    ));
                }
            }

            if let Some(security) = operation.get("security").and_then(|value| value.as_array()) {
                for requirement in security {
                    let Some(requirement) = requirement.as_object() else {
                        continue;
                    };
                    for name in requirement.keys() {
                        if !schemes.contains(&name.as_str()) {
                            return Err(format!(
                                "{method} {path}: security requirement `{name}` has no scheme"
                            ));
                        }
                    }
                }
            }

            let responses = operation
                .get("responses")
                .and_then(|value| value.as_object())
                .map(|responses| !responses.is_empty())
                .unwrap_or(false);
            if !responses {
                return Err(format!("{method} {path}: operation documents no responses"));
            }
        }
    }

    Ok(())
}
