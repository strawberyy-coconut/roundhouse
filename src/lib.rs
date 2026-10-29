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
pub mod prefix;
pub mod problem;
pub mod registry;
pub mod router;

/// Typed multipart extraction, re-exported from `roundhouse-multipart`.
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
pub use doc::{ApiError, ApiResponse, ApiSchema, PathNames, QueryParams};
pub use extract::{Extension, Form, Json, Path, Query, Validation};
pub use multipart::{FilePart, Multipart, MultipartForm, MultipartProblem, TempFile};
pub use problem::Problem;
pub use roundhouse_schema::{Param, ParamIn, PathValidate, ValidationError, ValidationErrors};
pub use router::{App, Built, DocumentRejections, ResolvedRoute};

pub use async_trait::async_trait;
pub use axum;
pub use http;
pub mod openapi {
    //! Convenience re-exports for configuring `App::openapi(..)`.
    pub use utoipa::openapi::{Info, OpenApi};
}
pub use inventory;
pub use regex;
pub use serde_json;
pub use utoipa;

#[cfg(feature = "macros")]
pub use roundhouse_macros::{
    ApiError, ApiResponse, ApiSchema, Multipart, PathParams, QueryParams, handler, mount, routes,
};

/// Start an app, scoped to the calling crate so linked dependencies' routes stay out.
#[macro_export]
macro_rules! app {
    ($state:expr) => {
        $crate::App::new($state).scope(module_path!())
    };
}

/// The crate's axum-shaped surface. Import this in every route module.
pub mod prelude {
    pub use crate::attachment::{Attachment, File};
    pub use crate::auth::{Auth, AuthScheme, AuthScopes, Authenticate, NoScopes};
    pub use crate::doc::PathNames;
    pub use crate::extract::{Extension, Form, Json, Path, Query};
    pub use crate::problem::Problem;
    pub use crate::validate::{ValidationError, ValidationErrors};
    pub use crate::{ApiError, ApiResponse, ApiSchema, app};
    pub use crate::{FilePart, Multipart, TempFile};
    #[cfg(feature = "macros")]
    pub use crate::{PathParams, QueryParams, handler, mount, routes};
    pub use axum::response::sse::{Event, KeepAlive, Sse};

    pub use axum::extract::{FromRequest, FromRequestParts, State};
    pub use axum::http::{HeaderMap, StatusCode};
    pub use axum::response::{IntoResponse, Response};
}
