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

/// Request extension inserted by a route entry with `validate = false`.
#[derive(Debug, Clone, Copy)]
pub struct SkipValidation;

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

/// Request extension that carries the app-wide validation settings.
#[derive(Debug, Clone, Copy)]
pub struct ValidationConfig {
    pub status: StatusCode,
    pub mode: Validation,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            mode: Validation::default(),
        }
    }
}

fn validation_config(extensions: &axum::http::Extensions) -> ValidationConfig {
    extensions
        .get::<ValidationConfig>()
        .copied()
        .unwrap_or_default()
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
pub trait Rejection {
    fn into_problem(self) -> Problem;
}

impl Rejection for Problem {
    fn into_problem(self) -> Problem {
        self
    }
}

macro_rules! rejection_via_status {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Rejection for $ty {
                fn into_problem(self) -> Problem {
                    rejection_problem(self.status(), self.body_text())
                }
            }
        )*
    };
}

rejection_via_status!(
    axum::extract::rejection::JsonRejection,
    axum::extract::rejection::FormRejection,
    axum::extract::rejection::QueryRejection,
    axum::extract::rejection::PathRejection,
    axum::extract::rejection::ExtensionRejection,
);

#[cfg(feature = "multipart")]
impl Rejection for axum::extract::multipart::MultipartRejection {
    fn into_problem(self) -> Problem {
        // `MultipartRejection` exposes its rejection through `IntoResponse`, not a structured
        // status/body API, so the status is preserved and the detail is generic.
        let response = self.into_response();
        let status = response.status();
        let status = if status == StatusCode::UNPROCESSABLE_ENTITY {
            StatusCode::BAD_REQUEST
        } else {
            status
        };
        Problem::from_rejection(
            status,
            status.canonical_reason().unwrap_or("Bad Request"),
            "multipart request was rejected".to_string(),
        )
    }
}

impl<T, S> FromRequest<S> for Mapped<T>
where
    T: FromRequest<S>,
    T::Rejection: Rejection,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Problem> {
        T::from_request(req, state)
            .await
            .map(Self)
            .map_err(Rejection::into_problem)
    }
}

impl<T, S> FromRequestParts<S> for Mapped<T>
where
    T: FromRequestParts<S>,
    T::Rejection: Rejection,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Problem> {
        T::from_request_parts(parts, state)
            .await
            .map(Self)
            .map_err(Rejection::into_problem)
    }
}

impl<T, S> OptionalFromRequest<S> for Mapped<T>
where
    T: OptionalFromRequest<S>,
    T::Rejection: Rejection,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Option<Self>, Problem> {
        T::from_request(req, state)
            .await
            .map(|value| value.map(Self))
            .map_err(Rejection::into_problem)
    }
}

impl<T, S> OptionalFromRequestParts<S> for Mapped<T>
where
    T: OptionalFromRequestParts<S>,
    T::Rejection: Rejection,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Option<Self>, Problem> {
        T::from_request_parts(parts, state)
            .await
            .map(|value| value.map(Self))
            .map_err(Rejection::into_problem)
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

/// Marks a request whose JSON body the router already validated from the raw bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BodyValidated;

/// The app's configured body limit, injected by the router so an extractor can enforce it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MaxBody(pub(crate) Option<usize>);

/// The validation policy for one request, read before a body is consumed.
#[derive(Debug, Clone, Copy)]
struct Policy {
    config: ValidationConfig,
    skip: bool,
}

impl Policy {
    fn read(parts: &Parts) -> Self {
        Self {
            config: validation_config(&parts.extensions),
            skip: parts.extensions.get::<SkipValidation>().is_some(),
        }
    }

    fn enabled(self) -> bool {
        !self.skip && self.config.mode != Validation::Off
    }

    fn problem(self, errors: ValidationErrors) -> Problem {
        validation_problem(errors, self.config.status)
    }
}

fn engine(parts: &Parts) -> Option<Arc<roundhouse_schema::engine::SchemaEngine>> {
    parts
        .extensions
        .get::<Arc<roundhouse_schema::engine::SchemaEngine>>()
        .cloned()
}

fn body_limit(parts: &Parts) -> usize {
    parts
        .extensions
        .get::<MaxBody>()
        .and_then(|limit| limit.0)
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
    T: DeserializeOwned + 'static,
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

    // The router's raw layer may have run already; it marks the request when it did.
    let already = parts.extensions.get::<BodyValidated>().is_some();
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
    let value = <axum::Json<T> as FromRequest<S>>::from_request(request, state)
        .await
        .map_err(Rejection::into_problem)?;
    Ok(Some(value))
}

impl<T, S> FromRequest<S> for Strict<axum::Json<T>>
where
    T: DeserializeOwned + 'static,
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
    T: DeserializeOwned + 'static,
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
    T: DeserializeOwned + serde::Serialize + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(request: axum::extract::Request, state: &S) -> Result<Self, Problem> {
        let (parts, body) = request.into_parts();
        let policy = Policy::read(&parts);
        let engine = engine(&parts);
        let request = axum::extract::Request::from_parts(parts, body);
        let axum::Form(value) = <axum::Form<T> as FromRequest<S>>::from_request(request, state)
            .await
            .map_err(Rejection::into_problem)?;
        if policy.enabled() {
            enforce_schema::<T>(&value, engine.as_ref(), policy.config.status)?;
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
            <axum::extract::Path<T> as FromRequestParts<S>>::from_request_parts(parts, state)
                .await
                .map_err(Rejection::into_problem)?;
        let policy = Policy::read(parts);
        if policy.enabled() {
            enforce_parts_schema::<T>(&value, engine(parts).as_ref(), policy.config.status)?;
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
        let value = <axum::extract::Path<T> as OptionalFromRequestParts<S>>::from_request_parts(
            parts, state,
        )
        .await
        .map_err(Rejection::into_problem)?;
        let Some(axum::extract::Path(value)) = value else {
            return Ok(None);
        };
        let policy = Policy::read(parts);
        if policy.enabled() {
            enforce_parts_schema::<T>(&value, engine(parts).as_ref(), policy.config.status)?;
            value.validate().map_err(|errors| policy.problem(errors))?;
        }
        Ok(Some(Strict(axum::extract::Path(value))))
    }
}

impl<T, S> FromRequestParts<S> for Strict<axum::extract::Query<T>>
where
    T: DeserializeOwned + crate::doc::QueryParams + crate::doc::PartsValue + 'static,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Problem> {
        let axum::extract::Query(value) =
            <axum::extract::Query<T> as FromRequestParts<S>>::from_request_parts(parts, state)
                .await
                .map_err(Rejection::into_problem)?;
        let policy = Policy::read(parts);
        if policy.enabled() {
            enforce_parts_schema::<T>(&value, engine(parts).as_ref(), policy.config.status)?;
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
            <axum::extract::Extension<T> as FromRequestParts<S>>::from_request_parts(parts, state)
                .await
                .map_err(Rejection::into_problem)?;
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
        // `OptionalFromRequestParts for Extension` is infallible.
        let value =
            <axum::extract::Extension<T> as OptionalFromRequestParts<S>>::from_request_parts(
                parts, state,
            )
            .await
            .unwrap_or(None);
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
        <crate::multipart::Multipart<F> as FromRequest<S>>::from_request(request, state)
            .await
            .map(Strict)
            .map_err(Rejection::into_problem)
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
        let skip = parts.extensions.get::<SkipValidation>().is_some();
        let config = validation_config(&parts.extensions);
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
        if !skip && config.mode != Validation::Off {
            if let Some(engine) = engine {
                let value = crate::doc::PartsValue::parts_value(&value);
                if let Some(result) = engine.validate_parts::<T>(&value) {
                    result.map_err(|errors| problem_from_validation(errors, config.status))?;
                }
            }
            value
                .validate()
                .map_err(|errors| problem_from_validation(errors, config.status))?;
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
