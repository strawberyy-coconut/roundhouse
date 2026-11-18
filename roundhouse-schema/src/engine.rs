//! Schema execution: compile the emitted document and validate values with it.
//!
//! The emitter and the enforcer are the same artifact. Given the components produced by
//! `#[derive(ApiSchema)]`, this module compiles one validator per type and maps the engine's
//! `instance_path` back onto the `ValidationError` shape clients already parse.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::validate::{ValidationError, ValidationErrors};

/// A compile failure: the document this crate emits is itself not a valid schema.
#[derive(Debug, Clone)]
pub struct SchemaError {
    pub message: String,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SchemaError {}

impl From<serde_json::Error> for SchemaError {
    fn from(error: serde_json::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

/// The contract-backed validator.
///
/// Every plan is keyed by the stable names `contract.json` recorded at build time: operation id
/// for request bodies and Rust type name for body/parts plans. No `TypeId` map is needed.
#[derive(Default)]
pub struct SchemaEngine {
    /// Build-time contract plans keyed by operation id.
    contract_bodies: HashMap<String, Arc<jsonschema::Validator>>,
    /// Content types for contract body plans, used to decide whether to install raw body
    /// validation without the startup document observations.
    contract_body_content_types: HashMap<String, String>,
    /// Build-time contract plans for body types that enforce themselves (`Strict<Json<T>>`).
    contract_types: HashMap<String, Arc<jsonschema::Validator>>,
    /// Build-time contract plans for path/query/form types, keyed by stable type name.
    contract_parts: HashMap<String, Arc<jsonschema::Validator>>,
    /// Whether this engine was compiled from a contract artifact.
    contract_backed: bool,
}

impl SchemaEngine {
    /// Compile the request validation plans from a build-time `contract.json`.
    ///
    /// The contract stores one schema table plus body plans keyed by operation id and type name,
    /// and parts plans keyed by type name. The schema table and both plan families share one
    /// compiled `$defs` map.
    pub fn from_contract_json(contract: &str) -> Result<Self, SchemaError> {
        let contract: Value = serde_json::from_str(contract)?;
        let validation = contract.get("validation").ok_or_else(|| SchemaError {
            message: "contract has no validation section; run `cargo roundhouse prepare`"
                .to_string(),
        })?;
        let defs = defs_from_schemas(
            validation
                .get("defs")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new())),
        )?;

        let mut engine = Self {
            contract_backed: true,
            ..Self::default()
        };
        if let Some(operations) = validation.pointer("/bodies/ops").and_then(Value::as_object) {
            for (operation_id, plan) in operations {
                if let Some(schema) = plan.get("schema") {
                    let validator = compile_schema(schema.clone(), &defs)?;
                    engine
                        .contract_bodies
                        .insert(operation_id.clone(), Arc::new(validator));
                }
                if let Some(content_type) = plan.get("content_type").and_then(Value::as_str) {
                    engine
                        .contract_body_content_types
                        .insert(operation_id.clone(), content_type.to_string());
                }
            }
        }
        if let Some(types) = validation
            .pointer("/bodies/types")
            .and_then(Value::as_object)
        {
            for (name, schema) in types {
                let validator = compile_schema(schema.clone(), &defs)?;
                engine
                    .contract_types
                    .insert(name.clone(), Arc::new(validator));
            }
        }
        if let Some(types) = validation
            .pointer("/parts/types")
            .and_then(Value::as_object)
        {
            for (name, schema) in types {
                let validator = compile_schema(schema.clone(), &defs)?;
                engine
                    .contract_parts
                    .insert(name.clone(), Arc::new(validator));
            }
        }
        Ok(engine)
    }

    /// Whether this engine was compiled from a contract artifact.
    pub fn is_contract_backed(&self) -> bool {
        self.contract_backed
    }

    /// The content type of a contract body plan for `operation_id`, if one exists.
    pub fn contract_body_content_type(&self, operation_id: &str) -> Option<&str> {
        self.contract_body_content_types
            .get(operation_id)
            .map(String::as_str)
    }

    /// Validate a request body against the contract plan for `operation_id`.
    pub fn validate_contract_body(
        &self,
        operation_id: &str,
        value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        self.contract_bodies
            .get(operation_id)
            .map(|validator| validate_with(validator, value))
    }

    /// Validate a body value that enforces its own schema during extraction, by stable type name.
    pub fn validate_contract_type(
        &self,
        type_name: &str,
        value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        self.contract_types
            .get(type_name)
            .map(|validator| validate_with(validator, value))
    }

    /// Validate path/query/form parts by stable type name.
    pub fn validate_contract_parts(
        &self,
        type_name: &str,
        value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        self.contract_parts
            .get(type_name)
            .map(|validator| validate_with(validator, value))
    }

    pub fn is_empty(&self) -> bool {
        self.contract_bodies.is_empty()
            && self.contract_types.is_empty()
            && self.contract_parts.is_empty()
            && self.contract_body_content_types.is_empty()
    }
}

/// Compile one type's schema. Useful for tests and the conformance harness.
#[cfg(feature = "build")]
pub fn compile<T: crate::schema::ApiSchema>() -> Result<jsonschema::Validator, SchemaError> {
    let mut components = utoipa::openapi::schema::Components::new();
    T::register(&mut components);
    let defs = defs_from_components(&components)?;
    let schema = serde_json::to_value(T::schema())?;
    compile_schema(schema, &defs)
}

/// Compile one schema value that may reference `#/components/schemas/…`.
pub fn compile_value(schema: &Value, defs: &Value) -> Result<jsonschema::Validator, SchemaError> {
    let defs = defs.as_object().cloned().unwrap_or_default();
    compile_schema(schema.clone(), &defs)
}

/// The component table as `$defs`, with references rewritten.
#[cfg(feature = "build")]
pub fn defs_from_components(
    components: &utoipa::openapi::schema::Components,
) -> Result<Map<String, Value>, SchemaError> {
    defs_from_schemas(serde_json::to_value(&components.schemas)?)
}

/// A serialized component table as `$defs`, with references rewritten.
fn defs_from_schemas(mut schemas: Value) -> Result<Map<String, Value>, SchemaError> {
    rewrite_refs(&mut schemas);
    match schemas {
        Value::Object(defs) => Ok(defs),
        _ => Ok(Map::new()),
    }
}

fn compile_schema(
    mut schema: Value,
    defs: &Map<String, Value>,
) -> Result<jsonschema::Validator, SchemaError> {
    rewrite_refs(&mut schema);
    if let Value::Object(object) = &mut schema {
        object
            .entry("$defs".to_string())
            .or_insert_with(|| Value::Object(defs.clone()));
    }
    jsonschema::draft202012::options()
        .offline()
        .should_validate_formats(true)
        .with_pattern_options(jsonschema::PatternOptions::regex())
        .build(&schema)
        .map_err(|error| SchemaError {
            message: error.to_string(),
        })
}

/// Validate one value and collect every violation, all at once.
pub fn validate_with(
    validator: &jsonschema::Validator,
    value: &Value,
) -> Result<(), ValidationErrors> {
    let mut errors = ValidationErrors::new();
    for error in validator.iter_errors(value) {
        collect_error(&error, &mut errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Flatten a union failure onto its most specific branch.
///
/// A tagged enum is a `oneOf`; the engine reports one root error with every branch's errors in
/// `context`. The branch with the fewest failures is the branch the value was trying to be, so its
/// errors carry the pointers and codes a client expects (`/title`, `min_length`) instead of a
/// root-level `oneOf` code.
fn collect_error(error: &jsonschema::ValidationError<'_>, errors: &mut ValidationErrors) {
    use jsonschema::error::ValidationErrorKind;

    let context = match error.kind() {
        ValidationErrorKind::AnyOf { context } | ValidationErrorKind::OneOfNotValid { context } => {
            Some(context)
        }
        _ => None,
    };
    if let Some(context) = context
        && let Some(best) = context.iter().min_by_key(|branch| branch.len())
        && !best.is_empty()
    {
        for nested in best {
            collect_error(nested, errors);
        }
        return;
    }
    errors.push(map_error(error));
}

/// Translate the engine's structured error into the shape clients already parse.
///
/// The keyword and its operand come from `ValidationErrorKind`, not from the message text, so the
/// `code` and `limit` stay stable when the engine changes its wording.
fn map_error(error: &jsonschema::ValidationError<'_>) -> ValidationError {
    use jsonschema::error::ValidationErrorKind;

    let pointer = error.instance_path().to_string();
    let (code, limit, message): (&str, Option<Value>, String) = match error.kind() {
        ValidationErrorKind::MinLength { limit } => (
            "min_length",
            Some(serde_json::json!(limit)),
            format!("must be at least {limit} character(s)"),
        ),
        ValidationErrorKind::MaxLength { limit } => (
            "max_length",
            Some(serde_json::json!(limit)),
            format!("must be at most {limit} character(s)"),
        ),
        ValidationErrorKind::Minimum { limit } => (
            "minimum",
            Some(limit.clone()),
            format!("must be greater than or equal to {limit}"),
        ),
        ValidationErrorKind::Maximum { limit } => (
            "maximum",
            Some(limit.clone()),
            format!("must be less than or equal to {limit}"),
        ),
        ValidationErrorKind::ExclusiveMinimum { limit } => (
            "exclusive_minimum",
            Some(limit.clone()),
            format!("must be greater than {limit}"),
        ),
        ValidationErrorKind::ExclusiveMaximum { limit } => (
            "exclusive_maximum",
            Some(limit.clone()),
            format!("must be less than {limit}"),
        ),
        ValidationErrorKind::MinItems { limit } => (
            "min_items",
            Some(serde_json::json!(limit)),
            format!("must contain at least {limit} item(s)"),
        ),
        ValidationErrorKind::MaxItems { limit } => (
            "max_items",
            Some(serde_json::json!(limit)),
            format!("must contain at most {limit} item(s)"),
        ),
        ValidationErrorKind::UniqueItems => ("unique_items", None, "items must be unique".into()),
        ValidationErrorKind::Pattern { pattern } => (
            "pattern",
            None,
            format!("must match the pattern `{pattern}`"),
        ),
        ValidationErrorKind::Format { format } => {
            let message = match format.as_str() {
                "email" | "idn-email" => "must be a valid email address".to_string(),
                "uuid" => "must be a valid UUID".to_string(),
                "uri" | "url" | "iri" => "must be a valid URL".to_string(),
                other => format!("must match format `{other}`"),
            };
            ("format", None, message)
        }
        ValidationErrorKind::Required { property } => {
            let name = property.as_str().unwrap_or_default();
            (
                "required",
                None,
                format!("missing required property `{name}`"),
            )
        }
        ValidationErrorKind::AdditionalProperties { unexpected } => (
            "additional_properties",
            None,
            format!("unexpected property/properties: {}", unexpected.join(", ")),
        ),
        ValidationErrorKind::Type { .. } => ("type", None, error.to_string()),
        ValidationErrorKind::Enum { options } => (
            "enum",
            Some(options.as_ref().clone()),
            "must be one of the enumerated values".to_string(),
        ),
        ValidationErrorKind::Constant { expected_value } => (
            "const",
            Some(expected_value.as_ref().clone()),
            format!("must be {expected_value}"),
        ),
        ValidationErrorKind::MultipleOf { multiple_of } => (
            "multiple_of",
            Some(serde_json::json!(multiple_of)),
            format!("must be a multiple of {multiple_of}"),
        ),
        ValidationErrorKind::Custom { keyword, message } => {
            (keyword.as_str(), None, message.to_string())
        }
        other => (other.keyword(), None, error.to_string()),
    };

    let mut mapped = ValidationError::new(pointer, code, message);
    if let Some(limit) = limit {
        mapped = mapped.with_limit(limit);
    }
    mapped
}

/// Whether a value is accepted. Used by the conformance harness.
pub fn is_valid(validator: &jsonschema::Validator, value: &Value) -> bool {
    validator.is_valid(value)
}

/// Rewrite OpenAPI component references to `$defs`, recursively.
pub fn rewrite_refs(value: &mut Value) {
    match value {
        Value::String(string) => {
            if let Some(rest) = string.strip_prefix("#/components/schemas/") {
                *string = format!("#/$defs/{rest}");
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_refs(item);
            }
        }
        Value::Object(object) => {
            for value in object.values_mut() {
                rewrite_refs(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn contract_engine_validates_operation_type_and_parts_plans() {
        let contract = serde_json::to_string(&json!({
            "validation": {
                "defs": {
                    "T": {
                        "type": "object",
                        "required": ["x"],
                        "properties": { "x": { "type": "integer" } }
                    }
                },
                "bodies": {
                    "ops": {
                        "op": {
                            "content_type": "application/json",
                            "schema": { "$ref": "#/components/schemas/T" }
                        }
                    },
                    "types": {
                        "T": { "$ref": "#/components/schemas/T" }
                    }
                },
                "parts": {
                    "types": {
                        "P": {
                            "type": "object",
                            "required": ["q"],
                            "properties": { "q": { "type": "string" } }
                        }
                    }
                }
            }
        }))
        .expect("contract serializes");

        let engine = SchemaEngine::from_contract_json(&contract).expect("contract compiles");
        assert!(engine.is_contract_backed());
        assert_eq!(
            engine.contract_body_content_type("op"),
            Some("application/json")
        );

        assert!(
            engine
                .validate_contract_body("op", &json!({}))
                .expect("operation plan")
                .is_err()
        );
        assert!(
            engine
                .validate_contract_body("op", &json!({ "x": 1 }))
                .expect("operation plan")
                .is_ok()
        );
        assert!(
            engine
                .validate_contract_type("T", &json!({ "x": "bad" }))
                .expect("type plan")
                .is_err()
        );
        assert!(
            engine
                .validate_contract_parts("P", &json!({}))
                .expect("parts plan")
                .is_err()
        );
    }
}
