//! The documentation half: schema derivation, parameter tables and response declarations.
//!
//! Most applications use this module through the derives re-exported at the crate root:
//! `#[derive(ApiSchema)]` for documented bodies and responses (with `ToSchema` for the shape),
//! plus `#[derive(QueryParams)]`, `#[derive(PathParams)]`, `#[derive(ApiResponse)]` and
//! `#[derive(ApiError)]` (and `#[derive(Multipart)]` with the `multipart` feature).
//!
//! The traits here are the extension points the macros use:
//!
//! - [`ExtractArg`] — how an axum extractor contributes parameters, a body, auth or failures, and
//!   what route-level validation it needs. Implement it for a custom extractor; no macro change is
//!   required.
//! - [`ApiSchema`] — the schema a documented body or response requires. It adds a named component
//!   and roundhouse-only patches and hooks on top of `utoipa::ToSchema`.
//! - [`ApiResponse`] — the documented responses a return type produces, plus its compile-time
//!   `SUCCESS` statuses; `ApiError` is re-exported from the macros.
//! - [`ResponseDoc`] and [`layer`] — hand-written responses and documented layer guarantees.
//! - [`validate_document`] — the structural check every build runs over the finished document.
//!
//! `axum::Json<T>`, `Form<T>` and JSON responses require `T: ApiSchema` for automatic
//! documentation. A plain `ToSchema` type remains available through an explicit
//! `@responses: status: Type` declaration. Component names follow utoipa: unique names stay
//! simple, and a collision is disambiguated with `#[schema(as = module::Type)]`.

pub mod build;
pub mod downgrade;
#[cfg(feature = "schema-engine")]
pub mod response;
pub mod validity;
pub use build::{build, build_with_options};
pub use validity::validate_document;
pub mod layer;

/// Schema derivation and constraint helpers, re-exported from `roundhouse-schema`.
pub use roundhouse_schema::schema;

use std::any::TypeId;
use utoipa::openapi::schema::Components;

pub use roundhouse_schema::{
    ApiSchema, Param, ParamIn, PartsDoc, PartsSchema, PartsValue, PathNames, PathValidate,
};

pub use utoipa;

mod arg;
mod body;
mod model;
mod params;
mod probe;
#[doc(hidden)]
pub mod registration;
mod response_doc;

pub use arg::*;
pub use body::*;
pub use model::*;
pub use params::*;
pub use probe::*;
pub use response_doc::*;

/// The documentation half a `#[handler]` attribute derives from the signature.
#[derive(Clone, Default)]
pub struct HandlerDocs {
    /// Operation summary from the handler's `@summary:` tag, unless the route entry overrides it.
    pub summary: Option<String>,
    /// Operation description from the handler's `@description:` tag.
    pub description: Option<String>,
    pub params: Vec<Param>,
    /// `@path:` descriptions, applied when the build fills a path capture.
    pub path_descriptions: Vec<(String, String)>,
    /// Path and query types the engine should compile for this operation.
    pub parts: Vec<PartsDoc>,
    pub body: Option<BodyDoc>,
    pub failures: Vec<ResponseDoc>,
    pub responses: Vec<ResponseDoc>,
    /// Responses declared explicitly by the handler's `@responses:` tag.
    pub declared_responses: Vec<ResponseDoc>,
    /// Whether `@responses(override):` allows a declared response to replace an inferred one.
    pub declared_response_override: bool,
    pub required_auth: Vec<crate::auth::AuthDoc>,
    pub optional_auth: Vec<crate::auth::AuthDoc>,
    pub schemes: Vec<crate::auth::SchemeDoc>,
    /// A body extractor the framework can see syntactically but cannot verify at runtime
    /// (`axum::Json`, `axum::Form`, `axum::extract::Multipart`). A route that validates the body
    /// through its shim clears this before the document is checked.
    pub unverified: bool,
    /// A response type the document cannot describe (`axum::Json<T>`, `axum::Html<T>` returned
    /// directly rather than through a derived response).
    pub unverified_response: bool,
    /// An extractor whose documented contribution the macro cannot infer (a custom `Tenant`,
    /// a bare `HeaderMap`, a typed header, ...).
    pub unverified_extractor: bool,
    /// A `Result<_, E>` whose `E` registered no documented responses.
    pub undocumented_error: Option<&'static str>,
    /// The schema of a scalar `Path<T>`; the parameter's name comes from the template.
    pub path_schema:
        Option<fn() -> Option<utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>>,
    /// The Rust type of the request body, so schema mode can compile the body's own schema even
    /// when it is composite (`Vec<T>`, `Option<T>`, maps, tuples).
    pub body_type: Option<fn() -> std::any::TypeId>,
    /// File fields declared without `max_bytes`; they fall back to the app-wide body limit.
    /// The build warns (and `strict` rejects) so the most permissive declaration is visible.
    pub unbounded_file_fields: Vec<&'static str>,
}

/// Implemented by the marker type `#[handler]` emits beside each handler.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a roundhouse handler marker",
    label = "add `#[handler]` to the handler function",
    note = "`routes!` needs the marker that `#[handler]` generates beside the handler; \
            a plain `async fn` has no handler documentation to derive"
)]
pub trait HandlerMeta: Send + Sync + 'static {
    /// The default router state when the `routes!` table does not declare `state = ...`.
    ///
    /// This is the first `State<T>` argument, the auth scheme's state, or `()`. A table's
    /// `state = ...` overrides it; the handler's `State<T>` arguments then use
    /// `T: FromRef<TableState>`.
    type State: 'static;

    /// Whether a default state was inferred from `State<T>` or `Auth<A>`.
    ///
    /// A table can declare a state for a stateless handler as well; this flag describes only the
    /// fallback derived from the signature.
    const HAS_STATE: bool;

    /// Capture names declared by the handler's `Path<T>` extractor, in template order.
    const PATH_NAMES: &'static [&'static str] = &[];

    /// Scheme names the handler requires with `Auth<A>` (`Option<Auth<A>>` does not count).
    ///
    /// A module-level guard for the same scheme is skipped: the extractor already performs the
    /// check during extraction, so the request must not be authenticated twice.
    const REQUIRED_AUTH: &'static [&'static str] = &[];

    /// The success statuses the return type guarantees, or `None` when the macro cannot classify
    /// it (a custom `ApiResponse`). `Some(&[])` means the macro knows there is no success
    /// response, so a route entry must declare one.
    const INFERRED_SUCCESS: Option<&'static [u16]> = None;

    /// Success statuses declared by the handler's `@responses:` block.
    ///
    /// The route-table compile-time check uses this when the return type cannot prove success.
    const DECLARED_SUCCESS: Option<&'static [u16]> = None;

    fn docs(components: &mut Components) -> HandlerDocs;

    /// Documentation with the route's validation policy.
    ///
    /// The generated implementation clears the body marker when the route's shim validated the
    /// body, so `strict()` does not reject a body the router enforces. A hand-written
    /// implementation can ignore the flag.
    fn docs_with(components: &mut Components, _validate: bool) -> HandlerDocs {
        Self::docs(components)
    }
}

pub fn handler_state<M: HandlerMeta>() -> Option<TypeId> {
    if M::HAS_STATE {
        Some(TypeId::of::<M::State>())
    } else {
        None
    }
}
