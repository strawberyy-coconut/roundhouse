//! Schema derivation, primitives, parameter schemas and schema patches.
//!
//! `ApiSchema` is the shape adapter used by the document builder; `PartsSchema`/`PartsValue`
//! describe path and query parameter types. The actual OpenAPI shape comes from
//! `utoipa::ToSchema`.

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, Schema};

use crate::validate::ValidationErrors;

pub trait ApiSchema {
    fn schema() -> RefOr<Schema>;

    /// Register this type (and anything it references) under its component name.
    fn register(_components: &mut Components) {}

    /// Run the `validate_with` hooks, with a JSON Pointer base. Constraint checks come from the
    /// emitted schema; this method only carries custom logic.
    fn validate_at(&self, _pointer: &str, _errors: &mut ValidationErrors) {}

    /// Run the `validate_with` hooks for this value.
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        self.validate_at("", &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Custom validation hooks for `Path<T>` extractors. Declarative constraints are enforced by the
/// emitted parameter schema.
pub trait PathValidate {
    fn validate_at(&self, _pointer: &str, _errors: &mut ValidationErrors) {}

    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        self.validate_at("", &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// A path or query type's declarative schema.
///
/// The derives build this from the same parameter table they publish, so the schema the engine
/// executes is the schema the document shows. Body types use `ToSchema` instead.
pub trait PartsSchema {
    fn parts_schema(components: &mut Components) -> RefOr<Schema>;
}

/// A path or query value as JSON, for the schema engine.
///
/// The derive implements this field by field, so a parts type does not need to derive `Serialize`
/// itself. Scalar `Path<T>` types use the implementations below.
pub trait PartsValue {
    fn parts_value(&self) -> serde_json::Value;
}

/// Build the object schema for a parts type from its documented parameters.
pub trait PathNames {
    const NAMES: &'static [&'static str] = &[];

    /// The schema of a scalar `Path<T>` whose name can only come from the template.
    fn scalar_schema() -> Option<RefOr<Schema>> {
        None
    }

    fn params() -> Vec<super::Param> {
        Vec::new()
    }
}

mod builders;
mod parts;
mod paths;
mod primitives;

pub use builders::*;
pub use parts::*;
