//! Schema derivation, constraints and validation.
//!
//! This crate has no route or axum dependency: the document half of roundhouse can be used by a
//! build script, an SDK or a test without compiling a server. The `roundhouse` facade re-exports
//! everything here, so user imports stay `roundhouse::…`.

#[cfg(feature = "schema-engine")]
pub mod engine;
#[cfg(not(feature = "schema-engine"))]
#[path = "engine_disabled.rs"]
pub mod engine;
pub mod schema;
pub mod validate;

#[cfg(feature = "schema-engine")]
pub use jsonschema;
pub use schema::{ApiSchema, PathNames, PathValidate};
pub use utoipa;
pub use validate::{ValidationError, ValidationErrors, pointer};

/// Where a parameter is read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamIn {
    Path,
    Query,
    Header,
    Cookie,
}

impl ParamIn {
    pub fn as_str(self) -> &'static str {
        match self {
            ParamIn::Path => "path",
            ParamIn::Query => "query",
            ParamIn::Header => "header",
            ParamIn::Cookie => "cookie",
        }
    }
}

/// One documented request parameter.
#[derive(Clone)]
pub struct Param {
    pub name: String,
    pub location: ParamIn,
    pub required: bool,
    pub schema: utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
    pub description: Option<String>,
    /// OpenAPI serialization style. `None` leaves the spec default for `location`.
    pub style: Option<utoipa::openapi::path::ParameterStyle>,
    /// Whether array/object values are exploded into repeated keys. `None` leaves the default.
    pub explode: Option<bool>,
}

impl Param {
    pub fn new(
        name: impl Into<String>,
        location: ParamIn,
        schema: utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
    ) -> Self {
        Self {
            name: name.into(),
            location,
            required: location == ParamIn::Path,
            schema,
            description: None,
            style: None,
            explode: None,
        }
    }

    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub fn style(mut self, style: utoipa::openapi::path::ParameterStyle) -> Self {
        self.style = Some(style);
        self
    }

    pub fn explode(mut self, explode: bool) -> Self {
        self.explode = Some(explode);
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}
