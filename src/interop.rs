//! Supported integration with utoipa.
//!
//! Application code should use `#[roundhouse::schema]` and `#[roundhouse(..)]`; it does not need
//! to import anything from this module. The re-exports exist for generated types, for projects
//! that already own `ToSchema` implementations, and for the explicit `as_utoipa`/`into_utoipa`
//! conversions on [`crate::openapi::Info`] and [`crate::openapi::OpenApi`].

pub use utoipa;
pub use utoipa::openapi::schema::Components;
pub use utoipa::{PartialSchema, ToSchema};

pub use crate::impl_from_utoipa;
