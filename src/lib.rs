//! # roundhouse
//!
//! An OpenAPI-first framework for axum: declarations become a `Router` and an OpenAPI document.
//!
//! ```ignore
//! mount!("/blog", tags = ["blog"]);
//!
//! routes! {
//!     get("/{id}", get, summary = "Read a post"),
//! }
//!
//! #[handler]
//! async fn get(Path(PostId { id }): Path<PostId>) -> Result<Json<Post>, Problem> { .. }
//! ```

pub mod attachment;
pub mod auth;
pub mod build_error;
pub mod doc;
pub mod extract;
pub mod openapi;
pub mod prefix;
pub mod problem;
pub mod registry;
pub mod router;

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
pub use extract::{Extension, Form, Json, Path, Query, Validation};
#[cfg(feature = "multipart-tempfile")]
pub use multipart::TempFile;
#[cfg(feature = "multipart")]
pub use multipart::{FilePart, Multipart, MultipartForm};
pub use problem::Problem;
pub use roundhouse_schema::{Param, ParamIn, PathValidate, ValidationError, ValidationErrors};
pub use router::{App, Built, DocumentRejections, MergeRecord, MountHandle, ResolvedRoute, Scope};

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

/// The crate's axum-shaped surface. Import this in every route module.
pub mod prelude {
    #[cfg(feature = "multipart-tempfile")]
    pub use crate::TempFile;
    pub use crate::attachment::{Attachment, File};
    pub use crate::auth::{Auth, AuthScheme, AuthScopes, Authenticate, Authorize, NoScopes};
    pub use crate::doc::PathNames;
    pub use crate::extract::{Extension, Form, Json, Path, Query};
    pub use crate::problem::Problem;
    pub use crate::validate::{ValidationError, ValidationErrors};
    pub use crate::{ApiError, ApiResponse, ApiSchema, app};
    #[cfg(feature = "multipart")]
    pub use crate::{FilePart, Multipart};
    #[cfg(feature = "macros")]
    pub use crate::{PathParams, QueryParams, handler, mount, routes};
    pub use axum::response::sse::{Event, KeepAlive, Sse};

    pub use axum::extract::{FromRequest, FromRequestParts, State};
    pub use axum::http::{HeaderMap, StatusCode};
    pub use axum::response::{IntoResponse, Response};
}
