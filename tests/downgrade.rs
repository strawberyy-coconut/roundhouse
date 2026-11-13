//! The best-effort OpenAPI 3.0 downgrade must preserve the constructs it claims to cover.

use roundhouse::doc::downgrade::to_openapi_30;
use roundhouse::interop::utoipa::openapi::OpenApi;
use serde_json::json;

fn document(schema: serde_json::Value) -> serde_json::Value {
    let openapi: OpenApi = serde_json::from_value(json!({
        "openapi": "3.1.0",
        "info": { "title": "Downgrade", "version": "0.1.0" },
        "paths": {},
        "components": {
            "schemas": { "Thing": schema }
        }
    }))
    .expect("valid OpenAPI fixture");
    to_openapi_30(&openapi)["components"]["schemas"]["Thing"].clone()
}

#[test]
fn nullable_enums_drop_the_null_member() {
    let thing = document(json!({
        "type": ["string", "null"],
        "description": "a nullable string",
        "enum": ["a", null]
    }));
    assert_eq!(thing["type"], "string");
    assert_eq!(thing["nullable"], true);
    assert_eq!(thing["description"], "a nullable string");
    assert_eq!(thing["enum"], json!(["a"]));
}

#[test]
fn nullable_one_of_keeps_sibling_metadata() {
    let thing = document(json!({
        "description": "nullable branch",
        "oneOf": [{ "type": "string" }, { "type": "null" }]
    }));
    assert_eq!(thing["type"], "string");
    assert_eq!(thing["nullable"], true);
    assert_eq!(thing["description"], "nullable branch");
    assert!(thing.get("oneOf").is_none());
}

#[test]
fn exclusive_bounds_keep_the_tighter_bound() {
    let lower = document(json!({
        "type": "number",
        "minimum": 1,
        "exclusiveMinimum": 5
    }));
    assert_eq!(lower["minimum"], 5);
    assert_eq!(lower["exclusiveMinimum"], true);

    let upper = document(json!({
        "type": "number",
        "maximum": 10,
        "exclusiveMaximum": 5
    }));
    assert_eq!(upper["maximum"], 5);
    assert_eq!(upper["exclusiveMaximum"], true);
}
