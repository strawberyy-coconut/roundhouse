use std::collections::BTreeMap;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

use super::schema;

/// A documented request body.
#[derive(Clone)]
pub struct BodyDoc {
    pub content_type: String,
    pub schema: RefOr<Schema>,
    /// Per-property encoding, e.g. the accepted content types of multipart file parts.
    pub encoding: BTreeMap<String, utoipa::openapi::encoding::Encoding>,
    pub required: bool,
    pub description: Option<String>,
}

/// A documented response header.
#[derive(Clone)]
pub struct HeaderDoc {
    pub name: String,
    pub schema: RefOr<Schema>,
    pub required: bool,
    pub append: bool,
}

/// One documented response.
#[derive(Clone)]
pub struct ResponseDoc {
    pub status: u16,
    pub description: String,
    pub content_type: Option<String>,
    pub schema: Option<RefOr<Schema>>,
    pub headers: Vec<HeaderDoc>,
}

impl ResponseDoc {
    pub fn new(status: u16, description: impl Into<String>) -> Self {
        Self {
            status,
            description: description.into(),
            content_type: None,
            schema: None,
            headers: Vec::new(),
        }
    }

    pub fn json(status: u16, schema: RefOr<Schema>) -> Self {
        Self {
            status,
            description: canonical_reason(status).to_string(),
            content_type: Some("application/json".to_string()),
            schema: Some(schema),
            headers: Vec::new(),
        }
    }

    pub fn problem(status: u16, title: &str) -> Self {
        Self {
            status,
            description: title.to_string(),
            content_type: Some("application/problem+json".to_string()),
            schema: Some(problem_schema()),
            headers: Vec::new(),
        }
    }

    /// The catch-all error response: a `Problem` at a status chosen at runtime.
    pub fn problem_default() -> Self {
        Self::problem(0, "Error")
    }

    /// A response whose status is chosen at runtime (`-> StatusCode`). The document says
    /// `default` with no body, which is the honest shape.
    pub fn default_response() -> Self {
        Self {
            status: 0,
            description: "Default Response".to_string(),
            content_type: None,
            schema: None,
            headers: Vec::new(),
        }
    }
}

/// The Problem body as a schema, for documented rejection responses.
pub fn problem_schema() -> RefOr<Schema> {
    use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, SchemaFormat, Type};

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

pub fn canonical_reason(status: u16) -> &'static str {
    if status == 0 {
        return "Default Response";
    }
    http::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("Unknown")
}
