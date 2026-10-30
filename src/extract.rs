//! Axum extractors with an optional `Problem` rejection.
//!
//! `Json`, `Form`, `Path` and `Extension` are re-exported from axum; `RepeatedQuery` is the one custom
//! parser (`serde_html_form`, so repeated keys work). Wrap any extractor in [`Strict<T>`] to
//! convert its rejection into the canonical [`Problem`] while keeping the inner type axum's.
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

/// A pure axum extractor with roundhouse's rejection contract.
///
/// `Strict<axum::Json<T>>` extracts with axum's [`axum::Json`] and converts its rejection into a
/// [`Problem`]; the inner type is untouched. Validation is still the route's job, so the wrapper
/// adds no constraint logic of its own.
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

impl<T, S> FromRequest<S> for Strict<T>
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

impl<T, S> FromRequestParts<S> for Strict<T>
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

impl<T, S> OptionalFromRequest<S> for Strict<T>
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

impl<T, S> OptionalFromRequestParts<S> for Strict<T>
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
