//! Build-time response inference.
//!
//! `cargo roundhouse prepare` analyzes handler bodies and writes an [`Inference`] document. The
//! contract builder folds those responses into the operation before the document is validated,
//! so an `impl IntoResponse` handler can be documented without a hand-written response enum.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::doc::{DocContext, ResponseDoc, SchemaContext, canonical_reason, schema};

/// The inferred responses for each operation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inference {
    /// Operation id → inferred responses.
    #[serde(default)]
    pub operations: BTreeMap<String, Vec<InferredResponse>>,
}

impl Inference {
    /// Parse an inference document.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Read an inference document from `path`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let json = std::fs::read_to_string(path)?;
        Self::from_json(&json).map_err(std::io::Error::other)
    }

    /// Serialize the inference document as pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// One inferred response for an operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InferredResponse {
    /// The status. `0` means OpenAPI's `default` response.
    pub status: u16,
    /// The response content type, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// A component name for the response body, when the analyzer could name it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Prose overrides. Inference normally leaves this empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The analyzer could not classify this outcome; it is a conservative `default Problem`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unknown: bool,
}

impl InferredResponse {
    /// Construct one inferred response.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            schema: None,
            description: None,
            unknown: false,
        }
    }

    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    pub fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Convert this inference into the document builder's response value.
    ///
    /// A body schema is only attached when its component is already present in the document;
    /// otherwise the status/content type is still documented, but the body stays unverified.
    pub(crate) fn to_response_doc(&self, context: &mut DocContext<'_>) -> ResponseDoc {
        if self.unknown {
            let mut response = ResponseDoc::problem_default();
            response.description = self
                .description
                .clone()
                .unwrap_or_else(|| "Unverified outcome".to_string());
            return response;
        }
        let description = self.description.clone().unwrap_or_else(|| {
            if self.status == 0 {
                "Default Response".to_string()
            } else {
                canonical_reason(self.status).to_string()
            }
        });
        let schema = self.schema.as_ref().and_then(|name| {
            context
                .components()
                .schemas
                .contains_key(name)
                .then(|| schema::r#ref(name.clone()))
        });
        let is_problem = self
            .content_type
            .as_deref()
            .is_some_and(|content_type| content_type == "application/problem+json");
        let schema = if schema.is_none() && is_problem {
            Some(crate::doc::problem_schema())
        } else {
            schema
        };
        ResponseDoc {
            status: self.status,
            description,
            content_type: self.content_type.clone(),
            schema,
            headers: Vec::new(),
        }
    }
}
