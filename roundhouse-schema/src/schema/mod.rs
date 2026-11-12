//! Schema derivation, primitives, parameter schemas and schema patches.
//!
//! `ApiSchema` is the shape adapter used by the document builder; [`Params`] is the single
//! path/query/form bound. The actual OpenAPI shape comes from `utoipa::ToSchema`.

use std::any::TypeId;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, Schema};

use crate::validate::ValidationErrors;

/// The result of claiming a component name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// The name was new and is now owned by this type.
    Claimed,
    /// This type (or its recursive registration) already claimed the name.
    Already,
    /// Another type owns the name. The caller must not overwrite the first schema.
    Collision,
    /// No document build is active; insert normally.
    Inactive,
}

/// The document build's schema registry.
///
/// `ApiSchema::register` receives this instead of a bare `Components`, so name-collision
/// detection is part of the build rather than a thread-local side channel. The implementation
/// lives in the facade; the schema crate only needs the operations.
pub trait SchemaContext {
    fn components(&mut self) -> &mut Components;

    /// Claim `name` for `type_name` and report whether it is safe to insert.
    fn claim(&mut self, name: &str, type_name: &'static str) -> Claim;

    /// Record a collision whose two Rust identities are not both known.
    fn record_collision(&mut self, name: &str, first: &'static str, second: &'static str);
}

/// A bare component table is a valid context for standalone schema registration (for example
/// the schema engine). It has no collision ledger, so every claim is [`Claim::Inactive`].
impl SchemaContext for Components {
    fn components(&mut self) -> &mut Components {
        self
    }

    fn claim(&mut self, _name: &str, _type_name: &'static str) -> Claim {
        Claim::Inactive
    }

    fn record_collision(&mut self, _name: &str, _first: &'static str, _second: &'static str) {}
}

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
    fn register(_context: &mut dyn SchemaContext) {}

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

/// A path or query type the schema engine should validate for this operation.
///
/// The document builder records the type once, and the engine compiles its parts schema for
/// runtime validation. Derived path/query types expose one; scalar path types have none.
#[derive(Clone, Copy)]
pub struct PartsDoc {
    pub type_id: fn() -> TypeId,
    pub schema: fn(&mut dyn SchemaContext) -> RefOr<Schema>,
}

impl PartsDoc {
    pub fn new<T: Params + 'static>() -> Self {
        Self {
            type_id: || TypeId::of::<T>(),
            schema: <T as Params>::parts_schema,
        }
    }
}

/// A path, query or form type: its documented parameters, schema, wire value and hooks.
///
/// This is the single bound `Path<T>`, `Query<T>` and `RepeatedQuery<T>` use. `PathParams` and
/// `QueryParams` derives emit it directly; scalar path types implement the scalar methods.
pub trait Params {
    const NAMES: &'static [&'static str] = &[];

    fn register(_context: &mut dyn SchemaContext) {}
    fn params(_context: &mut dyn SchemaContext) -> Vec<super::Param> {
        Vec::new()
    }
    fn scalar_schema() -> Option<RefOr<Schema>> {
        None
    }
    fn parts_schema(context: &mut dyn SchemaContext) -> RefOr<Schema> {
        crate::schema::object_from_params(Self::params(context))
    }
    fn parts_value(&self) -> serde_json::Value;
    fn validate_at(&self, _pointer: &str, _errors: &mut ValidationErrors) {}
    fn parts_doc() -> Option<PartsDoc> {
        None
    }

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

mod builders;
mod parts;
mod paths;
mod primitives;

pub use builders::*;
pub use parts::*;
