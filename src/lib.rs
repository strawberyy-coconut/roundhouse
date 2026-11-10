//! # roundhouse
//!
//! An OpenAPI-first framework for [axum]. You declare routes, parameters, schemas, auth and errors
//! once; roundhouse builds the `axum::Router` **and** the OpenAPI 3.1 document from those same
//! declarations. The module tree is the complete route set.
//!
//! [axum]: https://docs.rs/axum
//!
//! ## Quick start
//!
//! ```toml
//! [dependencies]
//! roundhouse = "0.2"
//! axum = "0.8"
//! serde = { version = "1", features = ["derive"] }
//! ```
//!
//! A route lives beside its handler in a `routes!` table. The table declares the module's prefix,
//! tags, security and layers, and composes child modules with `...module`:
//!
//! ```no_run
//! use roundhouse::prelude::*;
//!
//! #[derive(serde::Serialize, ToSchema)]
//! pub struct Hello {
//!     pub message: String,
//! }
//!
//! routes! {
//!     prefix = "/v1",
//!     tags = ["hello"],
//!     "/hello/{name}" => {
//!         get(hello),
//!     },
//! }
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
//! `GET /v1/hello/world` answers JSON, and `GET /openapi.json` describes the same operation,
//! including the `name` path parameter that `Path<String>` cannot name by itself.
//!
//! ## How it fits together
//!
//! 1. **Declare.** `routes!` emits one [`Module`] per invocation: its prefix, tags, layers and
//!    security guards, the routes declared in that table, and the children composed with
//!    `...module`. `#[handler]` derives the request, response and security documentation from the
//!    handler signature.
//! 2. **Build.** [`App::build`](router::App::build) walks the module tree into a [`Built`]:
//!    the `axum::Router`, the resolved route table and the OpenAPI document.
//!    [`App::try_build`](router::App::try_build) performs the same checks and returns a
//!    [`BuildError`] instead of panicking.
//! 3. **Enforce.** The published schema is executed for request validation, and declared security
//!    installs the matching guard. Rejections use the [`Problem`] body.
//!
//! ## Where to look next
//!
//! | Module | What it covers |
//! |---|---|
//! | [`prelude`] | everything a route module normally imports |
//! | [`router`] | `App`, `Built`, `Fragment`, `Module` |
//! | [`extract`] | `Json`, `Form`, `Path`, `Query`, `RepeatedQuery`, `Strict`, `Mapped`, `Validation` |
//! | [`doc`] | `ExtractArg`, `ApiSchema`, `ApiResponse`, parameter tables, layer docs |
//! | [`auth`] | `Auth<A>`, `AuthScheme`, `Authenticate`, `Authorize` |
//! | [`problem`] | the RFC 9457 `Problem` body |
//! | [`attachment`] | `Attachment`, `File`, SSE responses |
//! | [`openapi`] | `Info`, `OpenApi`, `Components` |
//! | [`build_error`] | `BuildError` and its source locations |
//! | [`schema`] | schema derivation and validation without axum |
//! | `multipart` | `Multipart<F>`, `FilePart`, `TempFile` (feature-gated) |
//!
//! The derive macros and `routes!` are re-exported at the crate root; [`prelude`] is the usual
//! import for application code. The repository's `docs/design/` notes explain the crate and
//! feature policy, the routing and document pipelines, validation and the macro diagnostics.
//!
//! ## Feature flags
//!
//! The default build includes the schema engine and the server runtime. The macros are always
//! part of the framework:
//!
//! | Feature | Enables |
//! |---|---|
//! | `schema-engine` | Execute the emitted JSON Schema for request and response validation (default) |
//! | `serve` | `App::serve`/`App::serve_with`, axum's `ConnectInfo<T>`, and the tokio server runtime (default) |
//! | `swagger` | Vendored Swagger UI assets for `App::swagger()` |
//! | `multipart` | `Multipart<F>`, `FilePart` and axum's multipart support |
//! | `multipart-tempfile` | `TempFile` (implies `multipart`) |
//! | `uuid`, `chrono`, `time`, `jiff`, `decimal` | `ToSchema`/`ApiSchema` support for those types |
//!
//! `ToSchema` is re-exported by [`prelude`], so `use roundhouse::prelude::*;` makes the derive
//! available. The prelude also re-exports the `utoipa` module used by the derives; importing
//! individual items instead uses `use roundhouse::utoipa;`.
//!
//! ## Examples
//!
//! The repository ships runnable examples for routing, module composition, layers, validation,
//! uploads, auth and utoipa interop (see the `examples/` directory). `examples/notes` is the
//! smallest and the best starting point.
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
pub mod response;
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
pub use doc::{
    ApiError, ApiResponse, ApiSchema, ExtractArg, PathNames, QueryParams, validate_document,
};
pub use extract::{
    Extension, Form, Json, Mapped, Path, Query, Rejection, RepeatedQuery, Strict, Validation,
};
#[cfg(feature = "multipart-tempfile")]
pub use multipart::TempFile;
#[cfg(feature = "multipart")]
pub use multipart::{FilePart, Multipart, MultipartForm};
pub use problem::Problem;
pub use registry::Module;
pub use response::{Created, NoContent};
pub use roundhouse_schema::{Param, ParamIn, PathValidate, ValidationError, ValidationErrors};
pub use router::{App, Built, DocumentRejections, Fragment, MergeRecord, ResolvedRoute};
pub use utoipa::{PartialSchema, ToSchema};

pub use axum;
pub use http;
#[cfg(feature = "schema-engine")]
pub use regex;
pub use serde_json;
pub use utoipa;

#[cfg(feature = "multipart")]
pub use roundhouse_macros::Multipart;
pub use roundhouse_macros::{
    ApiError, ApiResponse, ApiSchema, PathParams, QueryParams, handler, routes,
};

/// Start an app from the fragment declared in the calling module.
///
/// ```ignore
/// routes! { ...routes }
/// let app = roundhouse::app!(state);
/// ```
///
/// `app!(state, module)` roots the app at another module's fragment, which is how a library's
/// route tree is composed:
///
/// ```ignore
/// let app = roundhouse::app!(state, other_crate::routes);
/// ```
///
/// Both forms are equivalent to `App::new(state).fragment(..)`; the builder form is available
/// when the root is chosen at runtime.
#[macro_export]
macro_rules! app {
    ($state:expr) => {
        $crate::App::new($state).fragment(__roundhouse_fragment())
    };
    ($state:expr, $($module:ident)::+) => {
        $crate::App::new($state).fragment($($module)::+::__roundhouse_fragment())
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
    pub use crate::doc::{ExtractArg, PathNames};
    pub use crate::extract::{Extension, Form, Json, Mapped, Path, Query, RepeatedQuery, Strict};
    pub use crate::problem::Problem;
    pub use crate::response::{Created, NoContent};
    pub use crate::utoipa;
    pub use crate::validate::{ValidationError, ValidationErrors};
    pub use crate::{ApiError, ApiResponse, ApiSchema, ToSchema, app};
    #[cfg(feature = "multipart")]
    pub use crate::{FilePart, Multipart};
    pub use crate::{PathParams, QueryParams, handler, routes};
    pub use axum::response::sse::{Event, KeepAlive, Sse};

    pub use axum::extract::{FromRequest, FromRequestParts, State};
    pub use axum::http::{HeaderMap, StatusCode};
    pub use axum::response::{IntoResponse, Response};
}
