//! The schema engine when the `schema-engine` feature is off.
//!
//! The public surface stays the same so downstream code compiles without `jsonschema`; building
//! an engine returns the same error a failed compilation would, naming the feature to enable.

use serde_json::Value;

use crate::validate::ValidationErrors;

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

fn feature_error() -> SchemaError {
    SchemaError {
        message: "schema execution is disabled; enable the `schema-engine` feature".to_string(),
    }
}

/// A placeholder engine: nothing can be compiled without the feature.
#[derive(Default)]
pub struct SchemaEngine;

impl SchemaEngine {
    pub fn from_components(
        _components: &utoipa::openapi::schema::Components,
        _types: impl IntoIterator<Item = (std::any::TypeId, String)>,
    ) -> Result<Self, SchemaError> {
        Err(feature_error())
    }

    /// Validate a value against the compiled type. `None` means the type was not documented.
    pub fn validate<T: 'static>(&self, _value: &Value) -> Option<Result<(), ValidationErrors>> {
        None
    }

    pub fn is_empty(&self) -> bool {
        true
    }
}
