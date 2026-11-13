//! RFC 9457 `application/problem+json` responses.
//!
//! [`Problem`] is the error body used by roundhouse: malformed JSON, a wrong `Content-Type`, a
//! missing parameter, a failed constraint and an unmatched route all use this shape, so a client
//! needs one parser. A `Result<_, Problem>` documents a `default` problem response alongside the
//! success response.

use axum::body::Body;
use axum::response::{IntoResponse, Response};
use http::{StatusCode, header};
use roundhouse_schema::{ValidationError, ValidationErrors, schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, Schema, SchemaFormat, Type};

/// The one error body the framework produces.
///
/// Malformed JSON, a wrong `Content-Type`, a missing query parameter and a failed constraint all
/// arrive in this shape, so a client needs exactly one parser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    #[serde(rename = "type", default = "default_type")]
    pub type_uri: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ValidationError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

fn default_type() -> String {
    "about:blank".to_string()
}

impl Problem {
    pub fn new(status: StatusCode, title: impl Into<String>) -> Self {
        Self {
            type_uri: "about:blank".to_string(),
            title: title.into(),
            status: status.as_u16(),
            detail: None,
            errors: Vec::new(),
            instance: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn type_uri(mut self, uri: impl Into<String>) -> Self {
        self.type_uri = uri.into();
        self
    }

    pub fn instance(mut self, instance: impl Into<String>) -> Self {
        self.instance = Some(instance.into());
        self
    }

    /// The `400` (or configured status) produced by a failed constraint check.
    pub fn from_validation(errors: ValidationErrors) -> Self {
        Self {
            type_uri: "https://roundhouse.rs/problems/validation".to_string(),
            title: "Validation failed".to_string(),
            status: StatusCode::BAD_REQUEST.as_u16(),
            detail: None,
            errors: errors.into_vec(),
            instance: None,
        }
    }

    /// The same validation failure at the status the app configured (400 by default).
    pub fn from_validation_with_status(errors: ValidationErrors, status: StatusCode) -> Self {
        let mut problem = Self::from_validation(errors);
        problem.status = status.as_u16();
        problem
    }

    /// Convert an extractor rejection into the common shape.
    pub fn from_rejection(status: StatusCode, title: &str, detail: String) -> Self {
        Self::new(status, title).detail(detail)
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "Not Found").detail(detail)
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").detail(detail)
    }

    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {}", self.title, detail),
            None => f.write_str(&self.title),
        }
    }
}

impl std::error::Error for Problem {}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = self.status();
        let bytes = serde_json::to_vec(&self).unwrap_or_else(|_| {
            br#"{"type":"about:blank","title":"Internal Server Error","status":500}"#.to_vec()
        });
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/problem+json")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

impl From<ValidationErrors> for Problem {
    fn from(value: ValidationErrors) -> Self {
        Self::from_validation(value)
    }
}

/// The Problem body as an OpenAPI schema, for the document builder.
pub fn problem_schema_ref() -> RefOr<Schema> {
    let error = ObjectBuilder::new()
        .schema_type(Type::Object)
        .property("pointer", schema::string())
        .property("code", schema::string())
        .property("message", schema::string())
        .property("limit", schema::any_value())
        .required("pointer")
        .required("code")
        .required("message")
        .build();

    ObjectBuilder::new()
        .schema_type(Type::Object)
        .property(
            "type",
            schema::string_with_format(SchemaFormat::Custom("uri".to_string())),
        )
        .property("title", schema::string())
        .property(
            "status",
            ObjectBuilder::new()
                .schema_type(Type::Integer)
                .minimum(Some(100u16))
                .maximum(Some(599u16)),
        )
        .property("detail", schema::nullable(schema::string()))
        .property("instance", schema::nullable(schema::string()))
        .property("errors", ArrayBuilder::new().items(error))
        .required("type")
        .required("title")
        .required("status")
        .into()
}

/// The Problem body as JSON Schema, for callers that emit it directly.
///
/// It is the serialized [`problem_schema_ref`], so the document and the standalone schema cannot
/// drift apart.
pub fn problem_schema() -> Value {
    serde_json::to_value(problem_schema_ref()).expect("the Problem schema serializes")
}

pub fn problem_value(status: u16, title: &str) -> Value {
    serde_json::json!({ "status": status, "title": title })
}
