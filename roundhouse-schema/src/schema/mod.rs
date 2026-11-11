//! Schema derivation, primitives, parameter schemas and schema patches.
//!
//! `ApiSchema` is the shape adapter used by the document builder; `PartsSchema`/`PartsValue`
//! describe path and query parameter types. The actual OpenAPI shape comes from
//! `utoipa::ToSchema`.

use std::any::TypeId;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, Schema};

use crate::validate::ValidationErrors;

pub trait ApiSchema {
    fn schema() -> RefOr<Schema>;

    /// The type's configured description, used as the default prose for a JSON response body.
    ///
    /// `#[derive(ApiSchema)]` reads `#[schema(description = "..")]` first, then the type's
    /// `@description:` doc tag. Wrapper types such as `Vec<T>` and `Option<T>` forward the inner
    /// description.
    fn description() -> Option<&'static str> {
        None
    }

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

/// A path or query type the schema engine should validate for this operation.
///
/// The document builder records the type once, and the engine compiles its parts schema for
/// runtime validation. `PathNames` and `QueryParams` expose one when the type has a
/// `PartsSchema`; scalar path types have none.
#[derive(Clone, Copy)]
pub struct PartsDoc {
    pub type_id: fn() -> TypeId,
    pub schema: fn(&mut Components) -> RefOr<Schema>,
}

impl PartsDoc {
    pub fn new<T: PartsSchema + 'static>() -> Self {
        Self {
            type_id: || TypeId::of::<T>(),
            schema: <T as PartsSchema>::parts_schema,
        }
    }
}

/// Build the object schema for a parts type from its documented parameters.
pub trait PathNames {
    const NAMES: &'static [&'static str] = &[];

    /// The schema of a scalar `Path<T>` whose name can only come from the template.
    fn scalar_schema() -> Option<RefOr<Schema>> {
        None
    }

    /// Register the components referenced by this path type's field schemas.
    ///
    /// `params()` has no component table, so the build calls this before it publishes the
    /// parameters. Scalar path types have nothing to register.
    fn register(_components: &mut Components) {}

    fn params() -> Vec<super::Param> {
        Vec::new()
    }

    /// The parts schema this path type participates in, if any.
    fn parts_doc() -> Option<PartsDoc> {
        None
    }
}

mod builders;
mod parts;
mod paths;
mod primitives;

pub use builders::*;
pub use parts::*;
