//! The application builder, route resolution and router assembly.
//!
//! [`App`] starts from application state and an explicit module tree. In application code, prefer
//! the [`app!`](crate::app!) macro: `app!(state)` roots the app at the fragment declared in the
//! calling module, while `app!(state, module)` names another module's fragment. Builder methods
//! configure layers, the document, validation and request limits. [`App::build`] walks the tree
//! into a [`Built`]:
//!
//! - [`Built::router`] — the ready `axum::Router`;
//! - [`Built::document`] and [`Built::openapi_json`] — the OpenAPI 3.1 document;
//! - [`Built::routes`] — the resolved table, with each declaration's source location.
//!
//! A single `routes!` table checks its own duplicate routes, duplicate operation ids and missing
//! success responses during expansion, so `cargo check` catches those. The composed tree is
//! resolved at build time: collisions across modules, prefix joins, state mismatches and the
//! final document need the aggregate view. Use [`App::try_build`] in a test or a check binary so
//! CI can fail without starting a server. [`App::into_router`] returns the ready `axum::Router<()>`
//! for embedding in a larger axum application.
//!
//! ```no_run
//! # use roundhouse::prelude::*;
//! # fn state() -> () {}
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # roundhouse::routes! {}
//! let built = roundhouse::app!(state()).build();
//! std::fs::write("target/openapi.json", built.openapi_json())?;
//! // built.router() is the axum router; built.routes() has the source locations.
//! # Ok(())
//! # }
//! ```

mod app;
mod assemble;
mod build;
mod built;
mod fragment;
mod resolve;

pub use app::App;
pub use built::{Built, ResolvedRoute};
pub use fragment::{DocumentRejections, Fragment, MergeRecord};
