//! The documentation half: schema derivation, parameter tables and response declarations.
//!
//! Most applications use this module through the derives re-exported at the crate root:
//! `#[derive(ApiSchema)]` for documented bodies (with `ToSchema` for the shape), plus
//! `#[derive(QueryParams)]`, `#[derive(PathParams)]` (and `#[derive(Multipart)]` with the
//! `multipart` feature).
//!
//! The traits here are the extension points the macros use:
//!
//! - `DocumentedResponse` — what a success type documents: status, content type, body schema and
//!   headers. Built-ins implement it, including `Json<T>`, `Status<CODE, T>`, `Empty<CODE>` and
//!   the generated `roundhouse::status` wrappers. A custom response type can implement it exactly
//!   the same way. There is no name-based fallback.
//! - `DocumentedError` — the failure counterpart. `Result<T, E>` combines both; `Problem` is one
//!   implementor, not a mandatory error type.
//! - `DocumentedIntoResponse` — the marker behind `.into_documented_response()`, which opts an
//!   opaque return site into documentation and erases it to [`Documented`].
//! - `ExtractDoc` — how an axum extractor contributes parameters, a body, auth or failures.
//!   Route-level validation lives on [`ExtractArg`](crate::ExtractArg); implement both for a
//!   custom extractor and no macro change is required.
//! - `ApiSchema` — the schema a documented body or response requires. It adds a named component
//!   and roundhouse-only patches and hooks on top of `utoipa::ToSchema`.
//! - `ResponseDoc` and `layer` — hand-written responses and documented layer guarantees.
//! - `validate_document` — the structural check every build runs over the finished document.
//!
//! Opaque handler returns (`impl IntoResponse`, `Result<impl IntoResponse, E>`) are documented
//! from the body; see [`#[handler]`](macro@crate::handler) for the inference rules and the
//! `?`/`DocumentedError` pattern.
//!
//! A custom documentation impl depends on types in this module, so it must be compiled only when
//! the `build` feature is on. Wrap it with `#[roundhouse::build_only]`; a production build that
//! compiles only `contract` then drops the impl instead of failing to resolve the types.
//!
//! `axum::Json<T>`, `Form<T>` and JSON responses require `T: ApiSchema` for automatic
//! documentation. A plain `ToSchema` type can be adapted with
//! `roundhouse::interop::impl_from_utoipa!` or documented by an explicit
//! `DocumentedResponse` impl. Component names follow utoipa: unique names stay
//! simple, and a collision is disambiguated with `#[schema(as = module::Type)]`.

#[cfg(feature = "build")]
pub mod build;
#[cfg(feature = "build")]
pub mod downgrade;
#[cfg(feature = "build")]
pub mod validity;
#[cfg(all(feature = "build", feature = "validation"))]
pub use crate::contract_response::ContractResponseValidator;
#[cfg(feature = "build")]
pub use build::{build, build_with_options};
#[cfg(feature = "build")]
pub use validity::validate_document;
#[cfg(feature = "build")]
pub mod layer;
#[cfg(not(feature = "build"))]
pub mod layer {
    //! Documentation-only layer metadata (compiled out without `build`).

    /// A documented layer guarantee.
    #[derive(Clone, Debug, Default)]
    pub struct LayerDoc;
}

/// Schema derivation and constraint helpers, re-exported from `roundhouse-schema`.
#[cfg(feature = "build")]
pub use roundhouse_schema::schema;

#[cfg(feature = "build")]
pub use roundhouse_schema::{ApiSchema, Claim, Param, ParamsDoc, PartsDoc, SchemaContext};
pub use roundhouse_schema::{ParamIn, Params, ValidateHook};
use std::any::TypeId;

#[cfg(feature = "build")]
mod arg;
#[cfg(feature = "build")]
mod body;
#[cfg(feature = "build")]
mod context;
#[cfg(feature = "build")]
mod model;
#[cfg(feature = "build")]
mod params;
#[cfg(feature = "build")]
pub(crate) mod probe;
#[cfg(feature = "build")]
#[doc(hidden)]
pub mod registration;
#[cfg(feature = "build")]
pub(crate) mod response_doc;

#[cfg(feature = "build")]
pub use arg::*;
#[cfg(feature = "build")]
pub use body::*;
#[cfg(feature = "build")]
pub use context::*;
#[cfg(feature = "build")]
pub use model::*;
#[cfg(feature = "build")]
pub use params::*;
#[cfg(feature = "build")]
pub use response_doc::*;

/// A documented response whose concrete type has been erased.
///
/// [`DocumentedIntoResponse::into_documented_response`] returns this type, so every branch of an
/// `impl IntoResponse` handler can unify on one concrete value while the build-time analyzer still
/// records the receiver's [`DocumentedResponse`] type at the call site. The wrapper owns the
/// already-built [`axum::response::Response`] and only forwards it.
pub struct Documented(axum::response::Response);

impl Documented {
    /// Wrap an already-built response.
    pub fn new(response: axum::response::Response) -> Self {
        Self(response)
    }
}

impl axum::response::IntoResponse for Documented {
    fn into_response(self) -> axum::response::Response {
        self.0
    }
}

/// Documents a response and erases it to one concrete type.
///
/// `#[handler]` recognizes `.into_documented_response()` at a return site as an explicit request
/// to document the receiver type. The method erases the receiver to [`Documented`], so branches
/// returning different documented responses can share one `impl IntoResponse` return type. The
/// analyzer records the receiver's type at the call site; the erased value itself does not need a
/// static response shape. Under `build` the trait also requires [`DocumentedResponse`]; production
/// builds only require `IntoResponse`, so a handler can call the method in any build.
#[cfg(feature = "build")]
pub trait DocumentedIntoResponse: axum::response::IntoResponse + DocumentedResponse {
    fn into_documented_response(self) -> Documented
    where
        Self: Sized,
    {
        Documented(axum::response::IntoResponse::into_response(self))
    }
}

#[cfg(feature = "build")]
impl<T: axum::response::IntoResponse + DocumentedResponse> DocumentedIntoResponse for T {}

/// Production-build form of [`DocumentedIntoResponse`]: the documentation bound is compiled out,
/// but the marker method stays available so handler source does not need `cfg`.
#[cfg(not(feature = "build"))]
pub trait DocumentedIntoResponse: axum::response::IntoResponse {
    fn into_documented_response(self) -> Documented
    where
        Self: Sized,
    {
        Documented(axum::response::IntoResponse::into_response(self))
    }
}

#[cfg(not(feature = "build"))]
impl<T: axum::response::IntoResponse> DocumentedIntoResponse for T {}

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
#[cfg(feature = "build")]
#[derive(Clone, Debug, Default)]
pub struct Verification {
    body: bool,
    response: bool,
    extractor: bool,
    body_verified: bool,
    unbounded_file_fields: Vec<&'static str>,
}

#[cfg(feature = "build")]
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
#[cfg(feature = "build")]
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
    /// Stable key for the contract's body plan.
    pub(crate) body_type_name: Option<fn() -> &'static str>,
}

#[cfg(feature = "build")]
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

    /// Set the stable contract key for the body's validation plan.
    pub fn set_body_type_name(&mut self, body_type_name: fn() -> &'static str) {
        self.body_type_name = Some(body_type_name);
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

    /// Add the responses a declared type documents, using the declaration's status.
    ///
    /// The type owns the body, content type and headers; the declaration owns the status. An
    /// empty contribution becomes a status-only response.
    pub fn push_declared_responses(
        &mut self,
        status: u16,
        responses: impl IntoIterator<Item = ResponseDoc>,
    ) {
        let mut any = false;
        for response in responses {
            any = true;
            self.declared_responses.push(response.with_status(status));
        }
        if !any {
            self.push_declared_response(ResponseDoc::new(status, canonical_reason(status)));
        }
    }

    /// Add several typed response declarations.
    pub fn extend_declared_responses(&mut self, responses: impl IntoIterator<Item = ResponseDoc>) {
        self.declared_responses.extend(responses);
    }

    /// Mutate a typed response declaration by status.
    pub fn declared_response_mut(&mut self, status: u16) -> Option<&mut ResponseDoc> {
        self.declared_responses
            .iter_mut()
            .find(|response| response.status == status)
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

    /// The body's stable contract key, if one was inferred.
    pub fn body_type_name(&self) -> Option<fn() -> &'static str> {
        self.body_type_name
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
}

/// The operation documentation a `#[handler]` marker produces.
#[cfg(feature = "build")]
pub trait DocSource {
    fn operation(context: &mut DocContext<'_>) -> OperationDoc;
}

/// Runtime placeholders for the document types macros name in their generated route records.
///
/// Without `build`, no operation is ever described, but `routes!` still constructs a
/// `RouteSpec`. These stubs keep that record's shape identical while every doc-building body is
/// compiled out.
#[cfg(not(feature = "build"))]
#[derive(Clone, Copy)]
pub struct DocContext<'a>(std::marker::PhantomData<&'a ()>);

#[cfg(not(feature = "build"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct OperationDoc;

#[cfg(not(feature = "build"))]
pub trait DocSource {
    fn operation(_context: &mut DocContext<'_>) -> OperationDoc {
        OperationDoc
    }
}

#[cfg(not(feature = "build"))]
impl<T: ?Sized> DocSource for T {}

/// Marker for a type that is both a handler spec and a documentation source.
///
/// The blank implementation means `#[handler]` only emits [`HandlerSpec`] and [`DocSource`];
/// downstream bounds can still name the combined marker.
#[cfg(feature = "build")]
pub trait HandlerMeta: HandlerSpec + DocSource {}

#[cfg(feature = "build")]
impl<T: HandlerSpec + DocSource> HandlerMeta for T {}

pub fn handler_state<M: HandlerSpec>() -> Option<TypeId> {
    if M::HAS_STATE {
        Some(TypeId::of::<M::State>())
    } else {
        None
    }
}
