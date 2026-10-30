//! Schema execution: compile the emitted document and validate values with it.
//!
//! The emitter and the enforcer are the same artifact. Given the components produced by
//! `#[derive(ApiSchema)]`, this module compiles one validator per type and maps the engine's
//! `instance_path` back onto the `ValidationError` shape clients already parse.

use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};
use utoipa::openapi::schema::Components;

use crate::schema::ApiSchema;
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

/// One compiled validator per documented type, keyed by `TypeId`.
///
/// Bodies and parts are separate maps: the same Rust type can be a JSON body in one operation and
/// a query struct in another, where the two schemas differ.
#[derive(Default)]
pub struct SchemaEngine {
    bodies: HashMap<TypeId, Arc<jsonschema::Validator>>,
    parts: HashMap<TypeId, Arc<jsonschema::Validator>>,
}

impl SchemaEngine {
    /// Compile every `(TypeId, component name)` pair that has a component in the document.
    pub fn from_components(
        components: &Components,
        types: impl IntoIterator<Item = (TypeId, String)>,
    ) -> Result<Self, SchemaError> {
        Self::from_components_and_bodies(components, types, std::iter::empty())
    }

    /// Compile named components and inline body schemas in one pass.
    ///
    /// Composite bodies (`Vec<T>`, `Option<T>`, maps, tuples) have no component of their own; the
    /// document knows their inline schema, so schema mode compiles that instead of skipping them.
    pub fn from_components_and_bodies(
        components: &Components,
        types: impl IntoIterator<Item = (TypeId, String)>,
        bodies: impl IntoIterator<
            Item = (
                TypeId,
                utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
            ),
        >,
    ) -> Result<Self, SchemaError> {
        Self::from_components_bodies_and_parts(components, types, bodies, std::iter::empty())
    }

    /// Compile named components, inline body schemas and parts schemas in one pass.
    pub fn from_components_bodies_and_parts(
        components: &Components,
        types: impl IntoIterator<Item = (TypeId, String)>,
        bodies: impl IntoIterator<
            Item = (
                TypeId,
                utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
            ),
        >,
        parts: impl IntoIterator<
            Item = (
                TypeId,
                utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
            ),
        >,
    ) -> Result<Self, SchemaError> {
        let defs = defs_from_components(components)?;
        let mut engine = Self::default();
        for (type_id, name) in types {
            if !defs.contains_key(&name) {
                continue;
            }
            let validator = compile_ref(&defs, &name)?;
            engine.bodies.insert(type_id, Arc::new(validator));
        }
        for (type_id, schema) in bodies {
            let value = serde_json::to_value(&schema)?;
            let validator = compile_schema(value, &defs)?;
            engine.bodies.insert(type_id, Arc::new(validator));
        }
        for (type_id, schema) in parts {
            let value = serde_json::to_value(&schema)?;
            let validator = compile_schema(value, &defs)?;
            engine.parts.insert(type_id, Arc::new(validator));
        }
        Ok(engine)
    }

    /// Validate a body against its compiled type. `None` means the type was not documented.
    pub fn validate<T: 'static>(&self, value: &Value) -> Option<Result<(), ValidationErrors>> {
        self.bodies
            .get(&TypeId::of::<T>())
            .map(|validator| validate_with(validator, value))
    }

    /// Validate a path/query value against its compiled type.
    /// Validate a value against a type identified at runtime. Used by the request layer, which
    /// sees the document's `TypeId` rather than a generic parameter.
    pub fn validate_type(
        &self,
        type_id: TypeId,
        value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        self.bodies
            .get(&type_id)
            .map(|validator| validate_with(validator, value))
    }

    pub fn validate_parts<T: 'static>(
        &self,
        value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        self.parts
            .get(&TypeId::of::<T>())
            .map(|validator| validate_with(validator, value))
    }

    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty() && self.parts.is_empty()
    }
}

/// Compile one type's schema. Useful for tests and the conformance harness.
pub fn compile<T: ApiSchema>() -> Result<jsonschema::Validator, SchemaError> {
    let mut components = Components::new();
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
pub fn defs_from_components(components: &Components) -> Result<Map<String, Value>, SchemaError> {
    let mut value = serde_json::to_value(&components.schemas)?;
    rewrite_refs(&mut value);
    match value {
        Value::Object(defs) => Ok(defs),
        _ => Ok(Map::new()),
    }
}

fn compile_ref(
    defs: &Map<String, Value>,
    name: &str,
) -> Result<jsonschema::Validator, SchemaError> {
    let schema = serde_json::json!({ "$ref": format!("#/$defs/{name}") });
    compile_schema(schema, defs)
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
