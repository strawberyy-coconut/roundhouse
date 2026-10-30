//! Derive expansion for `ApiSchema` (roundhouse-only patches and hooks).

mod attrs;
mod expand;
mod schema_expr;

pub(crate) use attrs::schema_attrs;
pub use expand::expand;
pub(crate) use schema_expr::{field_schema_expr, field_validation};
