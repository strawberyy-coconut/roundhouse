//! Best-effort OpenAPI 3.0 downgrade of the 3.1 document.

use serde_json::{Map, Value};
use utoipa::openapi::OpenApi;

/// Convert a 3.1 document to a 3.0.3-shaped JSON value.
///
/// The transformations cover the constructs this crate emits: `type: [T, "null"]`,
/// `exclusiveMinimum/Maximum` numbers, `oneOf` with a null branch, `const`, and `examples`.
pub fn to_openapi_30(document: &OpenApi) -> Value {
    let mut value = serde_json::to_value(document).expect("OpenAPI document serializes");
    if let Value::Object(object) = &mut value {
        object.insert("openapi".to_string(), Value::String("3.0.3".to_string()));
        object.remove("webhooks");
        object.remove("jsonSchemaDialect");
        object.remove("$schema");
        object.remove("$self");
    }
    downgrade(&mut value);
    value
}

fn downgrade(value: &mut Value) {
    match value {
        Value::Object(object) => {
            downgrade_nullable(object);
            downgrade_exclusive(object);
            if let Some(Value::Array(mut examples)) = object.remove("examples")
                && let Some(first) = examples.drain(..).next()
            {
                object.entry("example").or_insert(first);
            }
            if let Some(constant) = object.remove("const") {
                object
                    .entry("enum")
                    .or_insert_with(|| Value::Array(vec![constant]));
            }
            for child in object.values_mut() {
                downgrade(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                downgrade(item);
            }
        }
        _ => {}
    }
}

fn downgrade_nullable(object: &mut Map<String, Value>) {
    // `type: [T, "null"]` -> `type: T, nullable: true`
    if let Some(Value::Array(types)) = object.get("type").cloned() {
        let had_null = types.iter().any(|value| value == "null");
        let remaining: Vec<Value> = types.into_iter().filter(|value| value != "null").collect();
        match remaining.len() {
            0 => {
                object.remove("type");
                if had_null {
                    object.insert("nullable".to_string(), Value::Bool(true));
                }
            }
            1 => {
                object.insert("type".to_string(), remaining.into_iter().next().unwrap());
                if had_null {
                    object.insert("nullable".to_string(), Value::Bool(true));
                }
            }
            _ => {
                object.insert("type".to_string(), Value::Array(remaining));
                if had_null {
                    object.insert("nullable".to_string(), Value::Bool(true));
                }
            }
        }
        if had_null {
            // OpenAPI 3.0 represents null through `nullable`, not an enum member.
            if let Some(Value::Array(values)) = object.get_mut("enum") {
                values.retain(|value| value != &Value::Null);
            }
        }
    }

    // `oneOf: [<schema>, {type: null}]` -> schema + nullable: true
    if let Some(Value::Array(items)) = object.get("oneOf").cloned()
        && items.len() == 2
    {
        let null_index = items
            .iter()
            .position(|item| item.get("type") == Some(&Value::String("null".to_string())));
        if let Some(null_index) = null_index {
            let other = items[1 - null_index].clone();
            if let Value::Object(mut other) = other {
                other.insert("nullable".to_string(), Value::Bool(true));
                // The original object may carry metadata such as `description` or `example`
                // beside the `oneOf`; keep it while replacing the schema branch.
                let mut merged = object.clone();
                merged.remove("oneOf");
                for (key, value) in other {
                    merged.insert(key, value);
                }
                *object = merged;
            }
        }
    }
}

fn downgrade_exclusive(object: &mut Map<String, Value>) {
    for (exclusive, bound, is_lower) in [
        ("exclusiveMinimum", "minimum", true),
        ("exclusiveMaximum", "maximum", false),
    ] {
        if let Some(Value::Number(number)) = object.get(exclusive).cloned() {
            let exclusive_value = number.as_f64();
            let bound_value = object.get(bound).and_then(Value::as_f64);
            let replace = match (bound_value, exclusive_value) {
                (Some(bound_value), Some(exclusive_value)) if is_lower => {
                    exclusive_value >= bound_value
                }
                (Some(bound_value), Some(exclusive_value)) => exclusive_value <= bound_value,
                _ => true,
            };
            if replace {
                object.insert(bound.to_string(), Value::Number(number));
            }
            object.insert(exclusive.to_string(), Value::Bool(true));
        }
    }
}
