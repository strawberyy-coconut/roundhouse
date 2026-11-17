use std::collections::BTreeMap;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

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
///
/// Response headers are documentation-only: the runtime decides whether to emit one. OpenAPI's
/// header object has no reliable "required" representation in the utoipa model this crate uses,
/// so a header's presence in this list describes a possible value, not a promise.
#[derive(Clone)]
pub struct HeaderDoc {
    pub name: String,
    pub schema: RefOr<Schema>,
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

    /// Rewrite the status, updating the description when it was the old canonical reason.
    ///
    /// A `response(status, Type)` declaration owns the status; the type owns the body, content
    /// type and headers. Keeping a description that was only "OK" or "Error" would be wrong
    /// after the status changes.
    pub fn with_status(mut self, status: u16) -> Self {
        let was_canonical =
            self.description == canonical_reason(self.status) || self.description == "Error";
        self.status = status;
        if was_canonical {
            self.description = canonical_reason(status).to_string();
        }
        self
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
///
/// The wire shape has one definition in [`crate::problem`], so the documented schema cannot
/// drift from the runtime body.
pub fn problem_schema() -> RefOr<Schema> {
    crate::problem::problem_schema_ref()
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
