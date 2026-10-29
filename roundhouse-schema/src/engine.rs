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
#[derive(Default)]
pub struct SchemaEngine {
    validators: HashMap<TypeId, Arc<jsonschema::Validator>>,
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
        let defs = defs_from_components(components)?;
        let mut validators = HashMap::new();
        for (type_id, name) in types {
            if !defs.contains_key(&name) {
                continue;
            }
            let validator = compile_ref(&defs, &name)?;
            validators.insert(type_id, Arc::new(validator));
        }
        for (type_id, schema) in bodies {
            let value = serde_json::to_value(&schema)?;
            let validator = compile_schema(value, &defs)?;
            validators.insert(type_id, Arc::new(validator));
        }
        Ok(Self { validators })
    }

    /// Validate a value against the compiled type. `None` means the type was not documented.
    pub fn validate<T: 'static>(&self, value: &Value) -> Option<Result<(), ValidationErrors>> {
        self.validators
            .get(&TypeId::of::<T>())
            .map(|validator| validate_with(validator, value))
    }

    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
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
        errors.push(ValidationError::new(
            error.instance_path().to_string(),
            error.kind().keyword(),
            error.to_string(),
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
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
