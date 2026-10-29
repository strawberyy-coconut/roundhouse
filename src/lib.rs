//! # roundhouse
//!
//! A thin layer over axum that turns declarations into a `Router` and an OpenAPI document.
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
pub mod doc;
pub mod extract;
pub mod multipart;
pub mod prefix;
pub mod problem;
pub mod registry;
pub mod router;
pub mod validate;

pub use attachment::{Attachment, AttachmentType, File};
pub use auth::{Auth, AuthDoc, AuthScheme, AuthScopes, Authenticate, NoScopes, SecuritySchemeKind};
pub use axum::response::sse::{Event, KeepAlive, Sse};
pub use doc::{ApiError, ApiResponse, ApiSchema, PathNames, QueryParams};
pub use extract::{Form, Json, Path, Query};
pub use multipart::{FilePart, Multipart, MultipartForm, Part, TempFile};
pub use problem::Problem;
pub use router::{App, Built, ResolvedRoute};
pub use validate::{ValidationError, ValidationErrors};

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
    pub use crate::extract::{Form, Json, Path, Query};
    pub use crate::problem::Problem;
    pub use crate::validate::{ValidationError, ValidationErrors};
    pub use crate::{ApiError, ApiResponse, ApiSchema, app};
    pub use crate::{FilePart, Multipart, TempFile};
    #[cfg(feature = "macros")]
    pub use crate::{PathParams, QueryParams, handler, mount, routes};
    pub use axum::response::sse::{Event, KeepAlive, Sse};

    pub use axum::extract::{Extension, FromRequest, FromRequestParts, State};
    pub use axum::http::{HeaderMap, StatusCode};
    pub use axum::response::{IntoResponse, Response};
}
