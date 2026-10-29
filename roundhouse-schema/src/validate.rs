//! Constraint validation and the errors carried by the RFC 9457 problem response.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One constraint violation.
///
/// `pointer` is a JSON Pointer into the request body (`/title`, `/tags/2`), so a client can map
/// the failure back to the exact value it sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationError {
    pub pointer: String,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<Value>,
}

impl ValidationError {
    pub fn new(
        pointer: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            pointer: pointer.into(),
            code: code.into(),
            message: message.into(),
            limit: None,
        }
    }

    pub fn with_limit(mut self, limit: impl Into<Value>) -> Self {
        self.limit = Some(limit.into());
        self
    }
}

/// Every constraint violation found in one request, collected rather than short-circuited.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ValidationErrors {
    pub errors: Vec<ValidationError>,
}

impl ValidationErrors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, error: ValidationError) {
        self.errors.push(error);
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.errors.len()
    }

    pub fn into_vec(self) -> Vec<ValidationError> {
        self.errors
    }

    /// Fold another set of errors into this one, shifting pointers under `prefix`.
    pub fn append_at(&mut self, prefix: &str, mut other: ValidationErrors) {
        for error in &mut other.errors {
            error.pointer = format!("{prefix}{}", error.pointer);
        }
        self.errors.append(&mut other.errors);
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{} (at {})", error.message, error.pointer)?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

/// Join a JSON Pointer segment, escaping `~` and `/` per RFC 6901.
pub fn pointer(base: &str, segment: &str) -> String {
    let segment = segment.replace('~', "~0").replace('/', "~1");
    if base.is_empty() {
        format!("/{segment}")
    } else {
        format!("{base}/{segment}")
    }
}
