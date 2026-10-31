//! Axum extractors with a `Problem` rejection.
//!
//! `Json`, `Form`, `Path` and `Extension` are re-exported from axum; `RepeatedQuery` is the one custom
//! parser (`serde_html_form`, so repeated keys work). The route shim maps the built-in extractors'
//! rejections to the canonical [`Problem`], so a handler names axum's types and still answers RFC 9457.
//! [`Mapped<T>`] maps a foreign extractor's rejection without enforcing anything; [`Strict<T>`]
//! enforces the operation's schema as part of extraction.
//! Validation is the route policy's job, not the extractor's: JSON bodies are checked from the raw
//! bytes before extraction, and parts are checked by the generated shim. `#[handler]` recognises
//! both forms by type:
//!
//! ```ignore
//! #[handler]
//! async fn create(
//!     State(app): State<AppState>,
//!     RepeatedQuery(page): RepeatedQuery<Page>,
//!     Json(input): Json<CreatePost>,
//! ) -> Result<Json<Post>, ApiError> { .. }
//! ```
//!
//! [`Validation`] selects the enforcer: the emitted JSON Schema, executed by the `schema-engine`
//! feature (enabled by default), or nothing. The schema is the only source of declarative
//! constraints; `validate_with` hooks run afterwards. Bodies are validated after deserialization,
//! so a violation carries a JSON Pointer into the request body.
//!
//! Validation is also a route policy. Every `routes!` entry is validated by default; the route's
//! generated shim runs the same checks for `axum`'s extractors (`axum::Json`, `Path`, `RepeatedQuery`,
//! `Form`), so a handler can stay on axum's types and still be enforced. `validate = false` opts
//! out. The framework's own extractors above validate during extraction and are not checked
//! twice.
//!
//! [`Extension`] is axum's stateless extension extractor, so a handler that only takes
//! `Extension<Arc<T>>` works with unit app state. [`Sse`](crate::Sse) and [`File`](crate::File)
//! live alongside the other response helpers.

use std::ops::{Deref, DerefMut};

use axum::extract::{FromRequest, FromRequestParts, OptionalFromRequest, OptionalFromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use std::sync::Arc;

use crate::problem::Problem;

use crate::validate::ValidationErrors;
pub use axum::Form;
pub use axum::Json;
pub use axum::extract::{Extension, Path, Query};

/// Which enforcer an app uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validation {
    /// Execute the emitted JSON Schema, then the `validate_with` hooks (the default when the
    /// `schema-engine` feature is enabled).
    Schema,
    /// Do not validate.
    Off,
}

#[allow(clippy::derivable_impls)]
impl Default for Validation {
    fn default() -> Self {
        #[cfg(feature = "schema-engine")]
        {
            Self::Schema
        }
        #[cfg(not(feature = "schema-engine"))]
        {
            Self::Off
        }
    }
}

/// Everything one request needs to know about enforcement.
///
/// One extension carries it: the app-wide settings, the route's opt-out, the body limit and
/// whether the raw JSON layer already ran. Each is a request-scoped fact, and the layers that
/// add them mutate this value instead of inserting a marker each.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ValidationState {
    pub(crate) status: StatusCode,
    pub(crate) mode: Validation,
    pub(crate) max_body: Option<usize>,
    pub(crate) skip: bool,
    pub(crate) body_validated: bool,
}

impl Default for ValidationState {
    fn default() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            mode: Validation::default(),
            max_body: None,
            skip: false,
            body_validated: false,
        }
    }
}

impl ValidationState {
    pub(crate) fn read(extensions: &axum::http::Extensions) -> Self {
        extensions
            .get::<Self>()
            .copied()
            .unwrap_or_default()
    }
}

/// An axum extractor whose rejection becomes a [`Problem`], with no schema enforcement.
///
/// `Mapped<axum::Json<T>>` extracts with axum's [`axum::Json`] and converts its rejection into a
/// [`Problem`]; the inner type is untouched. The framework cannot know how to hold a foreign
/// extractor to a schema, so none is applied. Use it for a third-party extractor, or when
/// extracting outside a route. For axum's built-in extractors use [`Strict`], which enforces.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mapped<T>(pub T);

impl<T> Mapped<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Mapped<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Mapped<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> From<T> for Mapped<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: IntoResponse> IntoResponse for Mapped<T> {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

/// Convert an extractor's rejection into the framework's error body.
///
/// `FromRequest` guarantees `IntoResponse`, so one blanket covers axum's built-ins and any
/// third-party extractor. The response already carries the status and the detail the client would
/// have seen; a 422 becomes 400 because the document promises a bad request.
pub trait Rejection {
    fn into_problem(self) -> impl std::future::Future<Output = Problem> + Send;
}

impl<T> Rejection for T
where
    T: IntoResponse + Send,
{
    // The explicit `+ Send` is the point: an `async fn` in a trait does not expose one, and the
    // route futures awaiting this must stay `Send`.
    #[allow(clippy::manual_async_fn)]
    fn into_problem(self) -> impl std::future::Future<Output = Problem> + Send {
        async move {
            let response = self.into_response();
            let status = normalize_rejection_status(response.status());
            let detail = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
                .filter(|detail| !detail.is_empty())
                .unwrap_or_else(|| {
                    status
                        .canonical_reason()
                        .unwrap_or("Bad Request")
                        .to_string()
                });
            Problem::from_rejection(
                status,
                status.canonical_reason().unwrap_or("Bad Request"),
                detail,
            )
        }
    }
}

fn normalize_rejection_status(status: StatusCode) -> StatusCode {
    if status == StatusCode::UNPROCESSABLE_ENTITY {
        StatusCode::BAD_REQUEST
    } else {
        status
    }
}

/// `T::from_request`, with the rejection converted to a `Problem` in one place.
#[allow(clippy::result_large_err)]
async fn from_request_problem<T, S>(
    request: axum::extract::Request,
    state: &S,
) -> Result<T, Problem>
where
    T: FromRequest<S> + Send,
    T::Rejection: Send,
{
    match T::from_request(request, state).await {
        Ok(value) => Ok(value),
        Err(rejection) => Err(rejection.into_problem().await),
    }
}

/// `T::from_request_parts`, with the rejection converted to a `Problem`.
#[allow(clippy::result_large_err)]
async fn from_request_parts_problem<T, S>(
    parts: &mut Parts,
    state: &S,
) -> Result<T, Problem>
where
    T: FromRequestParts<S> + Send,
    T::Rejection: Send,
{
    match T::from_request_parts(parts, state).await {
        Ok(value) => Ok(value),
        Err(rejection) => Err(rejection.into_problem().await),
    }
}

/// The optional forms, for `Option<Mapped<T>>` and friends.
#[allow(clippy::result_large_err)]
async fn optional_from_request_problem<T, S>(
    request: axum::extract::Request,
    state: &S,
) -> Result<Option<T>, Problem>
where
    T: OptionalFromRequest<S> + Send,
    T::Rejection: Send,
{
    match <T as OptionalFromRequest<S>>::from_request(request, state).await {
        Ok(value) => Ok(value),
        Err(rejection) => Err(rejection.into_problem().await),
    }
}

#[allow(clippy::result_large_err)]
async fn optional_from_request_parts_problem<T, S>(
    parts: &mut Parts,
    state: &S,
) -> Result<Option<T>, Problem>
where
    T: OptionalFromRequestParts<S> + Send,
    T::Rejection: Send,
{
    match <T as OptionalFromRequestParts<S>>::from_request_parts(parts, state).await {
        Ok(value) => Ok(value),
        Err(rejection) => Err(rejection.into_problem().await),
    }
}

impl<T, S> FromRequest<S> for Mapped<T>
where
    T: FromRequest<S> + Send,
    T::Rejection: Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Problem> {
        from_request_problem::<T, S>(req, state).await.map(Self)
    }
}

impl<T, S> FromRequestParts<S> for Mapped<T>
where
    T: FromRequestParts<S> + Send,
    T::Rejection: Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Problem> {
        from_request_parts_problem::<T, S>(parts, state)
            .await
            .map(Self)
    }
}

impl<T, S> OptionalFromRequest<S> for Mapped<T>
where
    T: OptionalFromRequest<S> + Send,
    T::Rejection: Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Option<Self>, Problem> {
        optional_from_request_problem::<T, S>(req, state)
            .await
            .map(|value| value.map(Self))
    }
}

impl<T, S> OptionalFromRequestParts<S> for Mapped<T>
where
    T: OptionalFromRequestParts<S> + Send,
    T::Rejection: Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Option<Self>, Problem> {
        optional_from_request_parts_problem::<T, S>(parts, state)
            .await
            .map(|value| value.map(Self))
    }
}

/// An axum extractor that enforces the operation's schema and answers `Problem`.
///
/// `Strict<axum::Json<T>>` buffers the body, validates it against the emitted schema, then
/// deserializes; `Strict<axum::extract::Path<T>>` validates the extracted parts. The inner type is
/// axum's, so the handler destructures it exactly as before. Writing it is an assertion that this
/// argument is held to its schema; the route policy (`validate = false`, `Validation::Off`) still
/// decides whether constraints run, and a `Result<Extractor, E>` argument is never wrapped.
///
/// The default path does not need this: the route shim maps axum's built-in extractors and the
/// router dispatches enforcement by the resolved type. Naming `Strict` makes enforcement part of
/// the signature instead.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Strict<T>(pub T);

impl<T> Strict<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Strict<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Strict<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> From<T> for Strict<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: IntoResponse> IntoResponse for Strict<T> {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

/// The validation policy for one request, read before a body is consumed.
#[derive(Debug, Clone, Copy)]
struct Policy {
    state: ValidationState,
}

impl Policy {
    fn read(parts: &Parts) -> Self {
        Self {
            state: ValidationState::read(&parts.extensions),
        }
    }

    fn enabled(self) -> bool {
        !self.state.skip && self.state.mode != Validation::Off
    }

    fn status(self) -> StatusCode {
        self.state.status
    }

    fn problem(self, errors: ValidationErrors) -> Problem {
        validation_problem(errors, self.state.status)
    }
}

fn engine(parts: &Parts) -> Option<Arc<roundhouse_schema::engine::SchemaEngine>> {
    parts
        .extensions
        .get::<Arc<roundhouse_schema::engine::SchemaEngine>>()
        .cloned()
}

fn body_limit(parts: &Parts) -> usize {
    ValidationState::read(&parts.extensions)
        .max_body
        .unwrap_or(2 * 1024 * 1024)
}

fn is_json(parts: &Parts) -> bool {
    parts
        .headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let value = value.split(';').next().unwrap_or(value).trim();
            value == "application/json" || value.ends_with("+json")
        })
}

fn too_large(limit: usize) -> Problem {
    Problem::from_rejection(
        StatusCode::PAYLOAD_TOO_LARGE,
        "Payload Too Large",
        format!("request body exceeds {limit} bytes"),
    )
}

fn malformed_json(detail: String) -> Problem {
    Problem::from_rejection(StatusCode::BAD_REQUEST, "Bad Request", detail)
}

/// Execute the emitted schema for a value that has already been deserialized.
///
/// Shared with the route shim: a form is not JSON on the wire, so the schema is applied to the
/// value serde produced, whichever path reached it.
#[allow(clippy::result_large_err)]
pub(crate) fn enforce_schema<T>(
    value: &T,
    engine: Option<&Arc<roundhouse_schema::engine::SchemaEngine>>,
    status: StatusCode,
) -> Result<(), Problem>
where
    T: serde::Serialize + 'static,
{
    if let Some(engine) = engine {
        let value =
            serde_json::to_value(value).map_err(|error| malformed_json(error.to_string()))?;
        if let Some(result) = engine.validate::<T>(&value) {
            result.map_err(|errors| validation_problem(errors, status))?;
        }
    }
    Ok(())
}

/// Execute the emitted parts schema for `Path`/`Query`. Shared with the route shim.
#[allow(clippy::result_large_err)]
pub(crate) fn enforce_parts_schema<T>(
    value: &T,
    engine: Option<&Arc<roundhouse_schema::engine::SchemaEngine>>,
    status: StatusCode,
) -> Result<(), Problem>
where
    T: crate::doc::PartsValue + 'static,
{
    if let Some(engine) = engine {
        let value = crate::doc::PartsValue::parts_value(value);
        if let Some(result) = engine.validate_parts::<T>(&value) {
            result.map_err(|errors| validation_problem(errors, status))?;
        }
    }
    Ok(())
}

/// Buffer, validate, then hand the bytes to axum's JSON extractor so its content-type and
/// deserialization rejections stay exactly as documented.
#[allow(clippy::result_large_err)]
async fn strict_json<T, S>(
    request: axum::extract::Request,
    state: &S,
    optional: bool,
) -> Result<Option<axum::Json<T>>, Problem>
where
    T: DeserializeOwned + Send + 'static,
    S: Send + Sync,
{
    let (parts, body) = request.into_parts();
    let policy = Policy::read(&parts);
    let engine = engine(&parts);
    let limit = body_limit(&parts);
    let bytes = axum::body::to_bytes(body, limit)
        .await
        .map_err(|_| too_large(limit))?;

    if optional && bytes.is_empty() {
        return Ok(None);
    }

    // The router's raw layer may have run already; it marks the state when it did.
    let already = policy.state.body_validated;
    if !already
        && policy.enabled()
        && is_json(&parts)
        && let Some(engine) = &engine
    {
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| malformed_json(error.to_string()))?;
        if let Some(result) = engine.validate::<T>(&value) {
            result.map_err(|errors| policy.problem(errors))?;
        }
    }

    let request = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
    let value = from_request_problem::<axum::Json<T>, S>(request, state).await?;
    Ok(Some(value))
}

impl<T, S> FromRequest<S> for Strict<axum::Json<T>>
where
    T: DeserializeOwned + Send + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(request: axum::extract::Request, state: &S) -> Result<Self, Problem> {
        let value = strict_json::<T, S>(request, state, false).await?;
        Ok(Strict(value.expect("a required body is always present")))
    }
}

impl<T, S> OptionalFromRequest<S> for Strict<axum::Json<T>>
where
    T: DeserializeOwned + Send + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Option<Self>, Problem> {
        Ok(strict_json::<T, S>(request, state, true).await?.map(Strict))
    }
}

impl<T, S> FromRequest<S> for Strict<axum::Form<T>>
where
    T: DeserializeOwned + serde::Serialize + Send + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(request: axum::extract::Request, state: &S) -> Result<Self, Problem> {
        let (parts, body) = request.into_parts();
        let policy = Policy::read(&parts);
        let engine = engine(&parts);
        let request = axum::extract::Request::from_parts(parts, body);
        let axum::Form(value) = from_request_problem::<axum::Form<T>, S>(request, state).await?;
        if policy.enabled() {
            enforce_schema::<T>(&value, engine.as_ref(), policy.status())?;
        }
        Ok(Strict(axum::Form(value)))
    }
}

impl<T, S> FromRequestParts<S> for Strict<axum::extract::Path<T>>
where
    T: DeserializeOwned + crate::PathValidate + crate::doc::PartsValue + Send + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Problem> {
        let axum::extract::Path(value) =
            from_request_parts_problem::<axum::extract::Path<T>, S>(parts, state).await?;
        let policy = Policy::read(parts);
        if policy.enabled() {
            enforce_parts_schema::<T>(&value, engine(parts).as_ref(), policy.status())?;
            value.validate().map_err(|errors| policy.problem(errors))?;
        }
        Ok(Strict(axum::extract::Path(value)))
    }
}

impl<T, S> OptionalFromRequestParts<S> for Strict<axum::extract::Path<T>>
where
    T: DeserializeOwned + crate::PathValidate + crate::doc::PartsValue + Send + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Option<Self>, Problem> {
        let value =
            optional_from_request_parts_problem::<axum::extract::Path<T>, S>(parts, state).await?;
        let Some(axum::extract::Path(value)) = value else {
            return Ok(None);
        };
        let policy = Policy::read(parts);
        if policy.enabled() {
            enforce_parts_schema::<T>(&value, engine(parts).as_ref(), policy.status())?;
            value.validate().map_err(|errors| policy.problem(errors))?;
        }
        Ok(Some(Strict(axum::extract::Path(value))))
    }
}

impl<T, S> FromRequestParts<S> for Strict<axum::extract::Query<T>>
where
    T: DeserializeOwned + crate::doc::QueryParams + crate::doc::PartsValue + Send + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Problem> {
        let axum::extract::Query(value) =
            from_request_parts_problem::<axum::extract::Query<T>, S>(parts, state).await?;
        let policy = Policy::read(parts);
        if policy.enabled() {
            enforce_parts_schema::<T>(&value, engine(parts).as_ref(), policy.status())?;
            value.validate().map_err(|errors| policy.problem(errors))?;
        }
        Ok(Strict(axum::extract::Query(value)))
    }
}

impl<T, S> FromRequestParts<S> for Strict<axum::extract::Extension<T>>
where
    T: Clone + Send + Sync + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Problem> {
        let value =
            from_request_parts_problem::<axum::extract::Extension<T>, S>(parts, state).await?;
        Ok(Strict(value))
    }
}

impl<T, S> OptionalFromRequestParts<S> for Strict<axum::extract::Extension<T>>
where
    T: Clone + Send + Sync + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Option<Self>, Problem> {
        let value =
            optional_from_request_parts_problem::<axum::extract::Extension<T>, S>(parts, state)
                .await?;
        Ok(value.map(Strict))
    }
}

#[cfg(feature = "multipart")]
impl<F, S> FromRequest<S> for Strict<crate::multipart::Multipart<F>>
where
    F: crate::multipart::MultipartForm + Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(request: axum::extract::Request, state: &S) -> Result<Self, Problem> {
        // `Multipart` already rejects with a `Problem`, so there is nothing to map.
        <crate::multipart::Multipart<F> as FromRequest<S>>::from_request(request, state)
            .await
            .map(Strict)
    }
}

fn rejection_problem(status: StatusCode, detail: String) -> Problem {
    // A malformed body is malformed input: the document promises 400, so the runtime uses 400
    // even where axum's own rejection defaults to 422.
    let status = if status == StatusCode::UNPROCESSABLE_ENTITY {
        StatusCode::BAD_REQUEST
    } else {
        status
    };
    Problem::from_rejection(
        status,
        status.canonical_reason().unwrap_or("Bad Request"),
        detail,
    )
}

/// RepeatedQuery extractor with `Problem`-shaped rejections and constraint validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepeatedQuery<T>(pub T);

impl<T> RepeatedQuery<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for RepeatedQuery<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for RepeatedQuery<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, S> FromRequestParts<S> for RepeatedQuery<T>
where
    T: DeserializeOwned + crate::doc::QueryParams + crate::doc::PartsValue + Send + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let state = ValidationState::read(&parts.extensions);
        let engine = parts
            .extensions
            .get::<Arc<roundhouse_schema::engine::SchemaEngine>>()
            .cloned();
        // `axum::extract::RepeatedQuery` uses `serde_urlencoded`, which cannot represent repeated keys.
        // The document describes arrays as `style: form, explode: true`, so the extractor must
        // accept `?tag=a&tag=b` rather than the last value or an error.
        let query = parts.uri.query().unwrap_or_default();
        let value: T = serde_html_form::from_str(query)
            .map_err(|error| rejection_problem(StatusCode::BAD_REQUEST, error.to_string()))?;
        if !state.skip && state.mode != Validation::Off {
            if let Some(engine) = engine {
                let value = crate::doc::PartsValue::parts_value(&value);
                if let Some(result) = engine.validate_parts::<T>(&value) {
                    result.map_err(|errors| problem_from_validation(errors, state.status))?;
                }
            }
            value
                .validate()
                .map_err(|errors| problem_from_validation(errors, state.status))?;
        }
        Ok(Self(value))
    }
}

fn problem_from_validation(errors: ValidationErrors, status: StatusCode) -> Problem {
    Problem::from_validation_with_status(errors, status)
}

/// The status a validation failure should use. `400` by default.
pub fn validation_problem(errors: ValidationErrors, status: StatusCode) -> Problem {
    Problem::from_validation_with_status(errors, status)
}
