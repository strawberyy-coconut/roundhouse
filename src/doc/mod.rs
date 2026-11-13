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
//! typed `response(status, Type)` declaration. Component names follow utoipa: unique names stay
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

/// The honesty markers an operation carries.
///
/// Keeping these together means a caller cannot accidentally treat "the body was verified" and
/// "the body is unverified" as independent facts; the builder owns the transitions.
#[derive(Clone, Debug, Default)]
pub struct Verification {
    body: bool,
    response: bool,
    extractor: bool,
    body_verified: bool,
    undocumented_error: Option<&'static str>,
    unbounded_file_fields: Vec<&'static str>,
}

impl Verification {
    /// A body extractor the framework cannot verify at runtime.
    pub fn body(&self) -> bool {
        self.body
    }

    /// A response type the document cannot describe.
    pub fn response(&self) -> bool {
        self.response
    }

    /// An extractor whose documented contribution the macro cannot infer.
    pub fn extractor(&self) -> bool {
        self.extractor
    }

    /// Whether every body argument enforces its own schema during extraction.
    pub fn body_verified(&self) -> bool {
        self.body_verified
    }

    /// A `Result<_, E>` whose `E` registered no documented responses.
    pub fn undocumented_error(&self) -> Option<&'static str> {
        self.undocumented_error
    }

    /// File fields declared without `max_bytes`.
    pub fn unbounded_file_fields(&self) -> &[&'static str] {
        &self.unbounded_file_fields
    }

    pub fn mark_body(&mut self) {
        self.body = true;
    }

    pub fn clear_body(&mut self) {
        self.body = false;
    }

    pub fn mark_response(&mut self) {
        self.response = true;
    }

    pub fn mark_extractor(&mut self) {
        self.extractor = true;
    }

    pub fn clear_extractor(&mut self) {
        self.extractor = false;
    }

    pub fn set_body_verified(&mut self, verified: bool) {
        self.body_verified = verified;
    }

    pub fn set_undocumented_error(&mut self, type_name: &'static str) {
        self.undocumented_error = Some(type_name);
    }

    pub fn extend_unbounded_file_fields(&mut self, names: impl IntoIterator<Item = &'static str>) {
        self.unbounded_file_fields.extend(names);
    }
}

/// The documentation half a `#[handler]` attribute derives from the signature.
///
/// The fields are crate-private: macro expansion and `doc = |docs| …` hooks mutate through the
/// builder methods below, so the operation's invariants do not depend on a caller remembering
/// which fields may be written together. Read accessors back
/// [`ResolvedRoute::docs`](crate::ResolvedRoute::docs).
#[derive(Clone, Default)]
pub struct OperationDoc {
    /// Operation summary from the handler's `@summary:` tag, unless the route entry overrides it.
    pub(crate) summary: Option<String>,
    /// Operation description from the handler's `@description:` tag.
    pub(crate) description: Option<String>,
    pub(crate) params: Vec<Param>,
    /// path descriptions, applied when the build fills a path capture.
    pub(crate) path_descriptions: Vec<(String, String)>,
    /// Path and query types the engine should compile for this operation.
    pub(crate) parts: Vec<PartsDoc>,
    pub(crate) body: Option<BodyDoc>,
    pub(crate) failures: Vec<ResponseDoc>,
    pub(crate) responses: Vec<ResponseDoc>,
    /// Responses declared explicitly by the handler's typed `response(..)` metadata.
    pub(crate) declared_responses: Vec<ResponseDoc>,
    /// Whether `override_response(..)` allows a declared response to replace an inferred one.
    pub(crate) declared_response_override: bool,
    /// Descriptions from `response_description(..)`, applied by status after
    /// inferred and declared responses are merged. Status `0` means OpenAPI's `default`.
    pub(crate) response_descriptions: Vec<(u16, String)>,
    pub(crate) required_auth: Vec<crate::auth::AuthDoc>,
    pub(crate) optional_auth: Vec<crate::auth::AuthDoc>,
    pub(crate) schemes: Vec<crate::auth::SchemeDoc>,
    /// Honesty markers: what this operation could not verify or describe.
    pub verification: Verification,
    /// The schema of a scalar `Path<T>`; the parameter's name comes from the template.
    pub(crate) path_schema:
        Option<fn() -> Option<utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>>,
    /// The Rust type of the request body, so schema mode can compile the body's own schema even
    /// when it is composite (`Vec<T>`, `Option<T>` or a map).
    pub(crate) body_type: Option<fn() -> std::any::TypeId>,
}

impl OperationDoc {
    /// Set the operation summary. A `doc` hook can use this to override the handler's tag.
    pub fn set_summary(&mut self, summary: impl Into<String>) {
        self.summary = Some(summary.into());
    }

    /// Set the operation description.
    pub fn set_description(&mut self, description: impl Into<String>) {
        self.description = Some(description.into());
    }

    /// Set the summary only when the handler supplied none.
    pub fn set_summary_if_absent(&mut self, summary: Option<String>) {
        if self.summary.is_none() {
            self.summary = summary;
        }
    }

    /// Set the description only when the handler supplied none.
    pub fn set_description_if_absent(&mut self, description: Option<String>) {
        if self.description.is_none() {
            self.description = description;
        }
    }

    /// Add one hand-written parameter.
    pub fn push_param(&mut self, param: Param) {
        self.params.push(param);
    }

    /// Add several parameters.
    pub fn extend_params(&mut self, params: impl IntoIterator<Item = Param>) {
        self.params.extend(params);
    }

    /// Describe a path capture by name.
    pub fn push_path_description(&mut self, name: impl Into<String>, text: impl Into<String>) {
        self.path_descriptions.push((name.into(), text.into()));
    }

    /// Register a path/query type the engine should compile.
    pub fn push_part(&mut self, part: PartsDoc) {
        self.parts.push(part);
    }

    /// Set the documented request body.
    pub fn set_body(&mut self, body: BodyDoc) {
        self.body = Some(body);
    }

    /// Set the body's Rust type so schema mode can compile a composite body.
    pub fn set_body_type(&mut self, body_type: fn() -> std::any::TypeId) {
        self.body_type = Some(body_type);
    }

    /// Set the schema of a scalar `Path<T>`.
    pub fn set_path_schema(
        &mut self,
        schema: fn() -> Option<utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>,
    ) {
        self.path_schema = Some(schema);
    }

    /// Add one rejection response.
    pub fn push_failure(&mut self, response: ResponseDoc) {
        self.failures.push(response);
    }

    /// Add several rejection responses.
    pub fn extend_failures(&mut self, responses: impl IntoIterator<Item = ResponseDoc>) {
        self.failures.extend(responses);
    }

    /// Add one inferred response.
    pub fn push_response(&mut self, response: ResponseDoc) {
        self.responses.push(response);
    }

    /// Add several inferred responses.
    pub fn extend_responses(&mut self, responses: impl IntoIterator<Item = ResponseDoc>) {
        self.responses.extend(responses);
    }

    /// Mutate the inferred response with `status`, if there is one.
    ///
    /// A `doc` hook uses this to adjust prose without reaching into the response vector.
    pub fn response_mut(&mut self, status: u16) -> Option<&mut ResponseDoc> {
        self.responses
            .iter_mut()
            .find(|response| response.status == status)
    }

    /// Add one typed response declaration.
    pub fn push_declared_response(&mut self, response: ResponseDoc) {
        self.declared_responses.push(response);
    }

    /// Add several typed response declarations.
    pub fn extend_declared_responses(&mut self, responses: impl IntoIterator<Item = ResponseDoc>) {
        self.declared_responses.extend(responses);
    }

    /// Allow typed declarations to replace an inferred response.
    pub fn set_declared_response_override(&mut self) {
        self.declared_response_override = true;
    }

    /// Override the prose of the response with `status`.
    pub fn push_response_description(&mut self, status: u16, description: impl Into<String>) {
        self.response_descriptions
            .push((status, description.into()));
    }

    /// Require an auth scheme.
    pub fn push_required_auth(&mut self, auth: crate::auth::AuthDoc) {
        self.required_auth.push(auth);
    }

    /// Document an optional auth scheme.
    pub fn push_optional_auth(&mut self, auth: crate::auth::AuthDoc) {
        self.optional_auth.push(auth);
    }

    /// Publish a security scheme used by this operation.
    pub fn push_scheme(&mut self, scheme: crate::auth::SchemeDoc) {
        self.schemes.push(scheme);
    }

    /// Require several auth schemes.
    pub fn extend_required_auth(&mut self, auth: impl IntoIterator<Item = crate::auth::AuthDoc>) {
        self.required_auth.extend(auth);
    }

    /// Document several optional auth schemes.
    pub fn extend_optional_auth(&mut self, auth: impl IntoIterator<Item = crate::auth::AuthDoc>) {
        self.optional_auth.extend(auth);
    }

    /// Publish several security schemes.
    pub fn extend_schemes(&mut self, schemes: impl IntoIterator<Item = crate::auth::SchemeDoc>) {
        self.schemes.extend(schemes);
    }

    /// The operation summary, if any.
    pub fn summary(&self) -> Option<&str> {
        self.summary.as_deref()
    }

    /// The operation description, if any.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The hand-written and inferred parameters.
    pub fn params(&self) -> &[Param] {
        &self.params
    }

    /// Path descriptions, in declaration order.
    pub fn path_descriptions(&self) -> &[(String, String)] {
        &self.path_descriptions
    }

    /// Path and query types registered for engine compilation.
    pub fn parts(&self) -> &[PartsDoc] {
        &self.parts
    }

    /// The documented request body, if any.
    pub fn body(&self) -> Option<&BodyDoc> {
        self.body.as_ref()
    }

    /// The rejection responses.
    pub fn failures(&self) -> &[ResponseDoc] {
        &self.failures
    }

    /// The inferred success responses.
    pub fn responses(&self) -> &[ResponseDoc] {
        &self.responses
    }

    /// The responses declared by typed metadata.
    pub fn declared_responses(&self) -> &[ResponseDoc] {
        &self.declared_responses
    }

    /// Whether a declared response may replace an inferred one.
    pub fn declared_response_override(&self) -> bool {
        self.declared_response_override
    }

    /// Response description overrides, in declaration order.
    pub fn response_descriptions(&self) -> &[(u16, String)] {
        &self.response_descriptions
    }

    /// The required auth schemes.
    pub fn required_auth(&self) -> &[crate::auth::AuthDoc] {
        &self.required_auth
    }

    /// The optional auth schemes.
    pub fn optional_auth(&self) -> &[crate::auth::AuthDoc] {
        &self.optional_auth
    }

    /// The published security schemes.
    pub fn schemes(&self) -> &[crate::auth::SchemeDoc] {
        &self.schemes
    }

    /// The scalar path schema, if one was inferred.
    pub fn path_schema(
        &self,
    ) -> Option<fn() -> Option<utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>> {
        self.path_schema
    }

    /// The body's Rust type, if schema mode should compile it.
    pub fn body_type(&self) -> Option<fn() -> std::any::TypeId> {
        self.body_type
    }
}

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

    /// Success statuses declared by the handler's typed response metadata.
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
