//! # roundhouse
//!
//! An OpenAPI-first framework for [axum]. You declare routes, parameters, schemas, auth and errors
//! once; roundhouse builds the `axum::Router` **and** the OpenAPI 3.1 document from those same
//! declarations. There is no separate wiring step and no hand-maintained spec.
//!
//! [axum]: https://docs.rs/axum
//!
//! ## Getting started
//!
//! ```toml
//! [dependencies]
//! roundhouse = "0.1"
//! axum = "0.8"
//! serde = { version = "1", features = ["derive"] }
//! ```
//!
//! A route lives beside its handler in a `routes!` table, and the module that owns a subtree
//! declares its prefix once in `mount!`:
//!
//! ```no_run
//! use roundhouse::prelude::*;
//!
//! #[derive(serde::Serialize, ToSchema, ApiSchema)]
//! pub struct Hello {
//!     pub message: String,
//! }
//!
//! mount!("/v1", tags = ["hello"]);
//! routes! { get("/hello/{name}", hello) }
//!
//! #[handler]
//! pub async fn hello(Path(name): Path<String>) -> Json<Hello> {
//!     Json(Hello { message: format!("hello {name}") })
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     roundhouse::app!(())
//!         .openapi(|| roundhouse::openapi::Info::new("Hello", "0.1.0"))
//!         .serve("0.0.0.0:3000")
//!         .await?;
//!     Ok(())
//! }
//! ```
//!
//! `GET /v1/hello/world` answers JSON, and `GET /openapi.json` describes the same route,
//! including the `name` path parameter that `Path<String>` cannot name by itself.
//!
//! ## How it fits together
//!
//! 1. **Declare.** `routes!` records each operation, `mount!` records a prefix plus the tags,
//!    layers and `Auth<A>` guards descendants inherit, and `#[handler]` derives the documentation
//!    half from the handler signature.
//! 2. **Build.** [`App::build`](router::App::build) joins every declaration in scope into a
//!    [`Built`]: the `axum::Router`, the resolved route table, and the document.
//!    A single table's duplicates, operation ids and missing success responses are rejected by
//!    the macro during expansion; collisions between tables, state mismatches and auth guards
//!    whose state does not match the app are found here.
//!    [`App::try_build`](router::App::try_build) reports the latter without panicking.
//! 3. **Enforce.** The extractors in [`extract`] enforce the constraints they publish, and
//!    [`auth`] enforces the security requirements it publishes. Rejections are [`Problem`].
//!
//! ## Where to look next
//!
//! | Module | What it covers |
//! |---|---|
//! | [`prelude`] | everything a route module normally imports |
//! | [`router`] | `App`, `Built`, `Scope`, `MountHandle` |
//! | [`extract`] | `Json`, `Form`, `Path`, `Query`, `Extension`, `Validation` |
//! | [`doc`] | `ApiSchema`, parameter tables, response docs, layer docs |
//! | [`auth`] | `Auth<A>`, `AuthScheme`, `Authenticate`, `Authorize` |
//! | [`problem`] | the RFC 9457 `Problem` body |
//! | [`attachment`] | `Attachment`, `File`, SSE responses |
//! | [`openapi`] | `Info`, `OpenApi`, `Components` |
//! | [`build_error`] | `BuildError` and its source locations |
//! | [`schema`] | schema derivation and validation without axum |
//! | `multipart` | `Multipart<F>`, `FilePart`, `TempFile` (feature-gated) |
//!
//! The derive macros and `routes!`/`mount!` are re-exported at the crate root; [`prelude`] is the
//! usual import for application code.
//!
//! ## Feature flags
//!
//! The default build includes the macros and the schema engine. Everything else is opt-in:
//!
//! | Feature | Enables |
//! |---|---|
//! | `multipart` | `Multipart<F>`, `FilePart` and axum's multipart support |
//! | `multipart-tempfile` | `TempFile` (implies `multipart`) |
//! | `schema-engine` | Execute the emitted JSON Schema for request and response validation (default) |
//! | `uuid`, `chrono`, `time`, `jiff`, `decimal` | `ToSchema`/`ApiSchema` support for those types |
//!
//! `ToSchema` is re-exported by [`prelude`], so the usual `use roundhouse::prelude::*;` makes the
//! derive available. The derived code refers to `utoipa` by name; importing the prelude brings
//! `utoipa` into scope, or use `use roundhouse::utoipa;` when importing items individually.
//!
//! ## Examples
//!
//! The repository ships runnable examples for routing, mounts and layers, validation, uploads,
//! auth and utoipa interop (see the `examples/` directory). `examples/notes` is the smallest and
//! the best starting point.
//!
//! ## Minimum supported Rust version
//!
//! Rust 1.88. The MSRV is checked in CI.

pub mod attachment;
pub mod auth;
pub mod build_error;
pub mod doc;
pub mod extract;
pub mod openapi;
#[doc(hidden)]
pub mod prefix;
pub mod problem;
#[doc(hidden)]
pub mod registry;
pub mod router;
#[doc(hidden)]
pub mod shim;

/// Typed multipart extraction, re-exported from `roundhouse-multipart`.
#[cfg(feature = "multipart")]
pub use roundhouse_multipart as multipart;
/// The schema half, re-exported from `roundhouse-schema`.
pub use roundhouse_schema as schema;
pub use roundhouse_schema::validate;

pub use attachment::{Attachment, AttachmentType, File};
pub use auth::{
    Auth, AuthDoc, AuthScheme, AuthScopes, Authenticate, Authorize, NoScopes, SecuritySchemeKind,
};
pub use axum::response::sse::{Event, KeepAlive, Sse};
pub use build_error::{BuildError, Source};
pub use doc::{ApiError, ApiResponse, ApiSchema, PathNames, QueryParams, validate_document};
pub use extract::{
    Extension, Form, Json, Path, Query, Rejection, RepeatedQuery, Strict, Validation,
};
#[cfg(feature = "multipart-tempfile")]
pub use multipart::TempFile;
#[cfg(feature = "multipart")]
pub use multipart::{FilePart, Multipart, MultipartForm};
pub use problem::Problem;
pub use roundhouse_schema::{Param, ParamIn, PathValidate, ValidationError, ValidationErrors};
pub use router::{App, Built, DocumentRejections, MergeRecord, MountHandle, ResolvedRoute, Scope};
pub use utoipa::{PartialSchema, ToSchema};

pub use axum;
pub use http;
pub use inventory;
pub use regex;
pub use serde_json;
pub use utoipa;

#[cfg(all(feature = "macros", feature = "multipart"))]
pub use roundhouse_macros::Multipart;
#[cfg(feature = "macros")]
pub use roundhouse_macros::{
    ApiError, ApiResponse, ApiSchema, PathParams, QueryParams, handler, mount, routes,
};

/// Start an app, scoped to the calling crate so linked dependencies' routes stay out.
///
/// The scope is the crate name, not the calling module: moving `main` into a submodule must not
/// silently produce an empty app.
#[macro_export]
macro_rules! app {
    ($state:expr) => {
        $crate::App::new($state).scope(env!("CARGO_CRATE_NAME"))
    };
}

/// The crate's axum-shaped surface: extractors, response types, routing macros and the traits a
/// route module needs. Import it with `use roundhouse::prelude::*;`.
///
/// It re-exports axum's `State`, `StatusCode`, `HeaderMap`, `IntoResponse`, `Response`, the
/// extractor traits and SSE types, so a handler module rarely needs a second `use axum::…` line.
pub mod prelude {
    #[cfg(feature = "multipart-tempfile")]
    pub use crate::TempFile;
    pub use crate::attachment::{Attachment, File};
    pub use crate::auth::{Auth, AuthScheme, AuthScopes, Authenticate, Authorize, NoScopes};
    pub use crate::doc::PathNames;
    pub use crate::extract::{Extension, Form, Json, Path, Query, RepeatedQuery, Strict};
    pub use crate::problem::Problem;
    pub use crate::utoipa;
    pub use crate::validate::{ValidationError, ValidationErrors};
    pub use crate::{ApiError, ApiResponse, ApiSchema, ToSchema, app};
    #[cfg(feature = "multipart")]
    pub use crate::{FilePart, Multipart};
    #[cfg(feature = "macros")]
    pub use crate::{PathParams, QueryParams, handler, mount, routes};
    pub use axum::response::sse::{Event, KeepAlive, Sse};

    pub use axum::extract::{FromRequest, FromRequestParts, State};
    pub use axum::http::{HeaderMap, StatusCode};
    pub use axum::response::{IntoResponse, Response};
}
