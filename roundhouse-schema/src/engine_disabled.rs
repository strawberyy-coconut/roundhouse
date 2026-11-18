//! The schema engine when the `validation` feature is off.
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
        message: "schema execution is disabled; enable the `validation` feature".to_string(),
    }
}

/// A placeholder engine: nothing can be compiled without the feature.
#[derive(Default)]
pub struct SchemaEngine;

impl SchemaEngine {
    pub fn from_contract_json(_contract: &str) -> Result<Self, SchemaError> {
        Err(feature_error())
    }

    pub fn is_contract_backed(&self) -> bool {
        false
    }

    pub fn contract_body_content_type(&self, _operation_id: &str) -> Option<&str> {
        None
    }

    pub fn validate_contract_body(
        &self,
        _operation_id: &str,
        _value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        None
    }

    pub fn validate_contract_type(
        &self,
        _type_name: &str,
        _value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        None
    }

    pub fn validate_contract_parts(
        &self,
        _type_name: &str,
        _value: &Value,
    ) -> Option<Result<(), ValidationErrors>> {
        None
    }

    pub fn is_empty(&self) -> bool {
        true
    }
}
