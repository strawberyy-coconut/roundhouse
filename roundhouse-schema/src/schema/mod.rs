//! Schema derivation, primitives, parameter schemas and schema patches.
//!
//! `ApiSchema` is the shape adapter used by the document builder; [`ParamsDoc`] extends the
//! runtime [`Params`](crate::params::Params) bound with parameter schemas. The actual OpenAPI
//! shape comes from `utoipa::ToSchema`.

pub use utoipa::openapi::RefOr;
pub use utoipa::openapi::path::ParameterStyle;
pub use utoipa::openapi::schema::{Components, Schema};

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
}

/// A path or query type the schema engine should validate for this operation.
///
/// The document builder records the type once, and the engine compiles its parts schema for
/// runtime validation. Derived path/query types expose one; scalar path types have none.
#[derive(Clone, Copy)]
pub struct PartsDoc {
    /// Stable key for the build-time contract. Type names are stable within one build, which is
    /// the lifetime that matters for the generated contract.
    pub type_name: fn() -> &'static str,
    pub schema: fn(&mut dyn SchemaContext) -> RefOr<Schema>,
}

impl PartsDoc {
    pub fn new<T: ParamsDoc + 'static>() -> Self {
        Self {
            type_name: || std::any::type_name::<T>(),
            schema: <T as ParamsDoc>::parts_schema,
        }
    }
}

/// The build-only half of a path, query or form type: its documented parameters and schema.
///
/// Runtime code bounds on [`Params`](crate::params::Params); the document builder additionally
/// requires this trait. `PathParams` and `QueryParams` derive both, but only `Params` survives a
/// production build.
pub trait ParamsDoc: crate::params::Params {
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
