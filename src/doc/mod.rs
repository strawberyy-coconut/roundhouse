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

pub use roundhouse_schema::{ApiSchema, Claim, Param, ParamIn, Params, PartsDoc, SchemaContext};
use std::any::TypeId;

pub use utoipa;

mod arg;
mod body;
mod context;
mod model;
mod params;
pub(crate) mod probe;
#[doc(hidden)]
pub mod registration;
pub(crate) mod response_doc;

pub use arg::*;
pub use body::*;
pub use context::*;
pub use model::*;
pub use params::*;
pub use response_doc::*;

/// Split public prose into an operation summary and description.
///
/// The first non-empty line is the summary; everything after the next blank line is the
/// description. This is the convention behind `routes!`'s `prose = include_str!(..)` option, so a
/// single documentation file can supply both fields without duplicating the text in doc tags.
pub fn split_prose(text: &'static str) -> (Option<&'static str>, Option<&'static str>) {
    let text = text.trim();
    if text.is_empty() {
        return (None, None);
    }
    let (summary, rest) = match text.split_once('\n') {
        Some((summary, rest)) => (summary.trim(), rest),
        None => (text, ""),
    };
    let description = rest.trim();
    let summary = (!summary.is_empty()).then_some(summary);
    let description = (!description.is_empty()).then_some(description);
    (summary, description)
}

/// The documentation half a `#[handler]` attribute derives from the signature.
#[derive(Clone, Default)]
pub struct OperationDoc {
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
    /// Descriptions from the handler's `@response-descriptions:` tag, applied by status after
    /// inferred and declared responses are merged. Status `0` means OpenAPI's `default`.
    pub response_descriptions: Vec<(u16, String)>,
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
    /// Whether every body argument enforces its own schema during extraction. The builder uses
    /// this and the route's validation policy to clear [`Self::unverified`] when appropriate.
    pub body_verified: bool,
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

/// Compatibility alias for the old name. New code should use [`OperationDoc`].
pub type HandlerDocs = OperationDoc;

/// The static facts a `#[handler]` marker exposes to `routes!`.
///
/// This is separate from [`DocSource`] so the route table can read state, captures, auth and
/// success statuses without generating the operation document.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a roundhouse handler marker",
    label = "add `#[handler]` to the handler function",
    note = "`routes!` needs the marker that `#[handler]` generates beside the handler; \
            a plain `async fn` has no handler documentation to derive"
)]
pub trait HandlerSpec: Send + Sync + 'static {
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
}

/// The operation documentation a `#[handler]` marker produces.
pub trait DocSource {
    fn operation(context: &mut DocContext<'_>) -> OperationDoc;
}

/// Marker for a type that is both a handler spec and a documentation source.
///
/// The blank implementation means `#[handler]` only emits [`HandlerSpec`] and [`DocSource`];
/// downstream bounds can still name the combined marker.
pub trait HandlerMeta: HandlerSpec + DocSource {}

impl<T: HandlerSpec + DocSource> HandlerMeta for T {}

pub fn handler_state<M: HandlerSpec>() -> Option<TypeId> {
    if M::HAS_STATE {
        Some(TypeId::of::<M::State>())
    } else {
        None
    }
}
