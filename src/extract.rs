//! The axum-shaped surface: extractors, responses and the prelude.

use std::ops::{Deref, DerefMut};

use axum::extract::{FromRequest, FromRequestParts};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::doc::ApiSchema;
use crate::problem::Problem;
use crate::validate::ValidationErrors;

/// Request extension inserted by a route entry with `validate = false`.
#[derive(Debug, Clone, Copy)]
pub struct SkipValidation;

/// Request extension that carries the app-wide validation failure status.
#[derive(Debug, Clone, Copy)]
pub struct ValidationConfig {
    pub status: StatusCode,
}

fn validation_status(extensions: &axum::http::Extensions) -> StatusCode {
    extensions
        .get::<ValidationConfig>()
        .map(|config| config.status)
        .unwrap_or(StatusCode::BAD_REQUEST)
}

/// JSON extractor and response with `Problem`-shaped rejections and constraint validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Json<T>(pub T);

impl<T> Json<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Json<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> From<T> for Json<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned + ApiSchema + Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let skip = req.extensions().get::<SkipValidation>().is_some();
        let status = validation_status(req.extensions());
        let value = axum::Json::<T>::from_request(req, state)
            .await
            .map_err(problem_from_json)?;
        if !skip {
            value
                .0
                .validate()
                .map_err(|errors| problem_from_validation(errors, status))?;
        }
        Ok(Self(value.0))
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

fn problem_from_json(rejection: axum::extract::rejection::JsonRejection) -> Problem {
    rejection_problem(rejection.status(), rejection.body_text())
}

/// Path extractor with `Problem`-shaped rejections and constraint validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Path<T>(pub T);

impl<T> Path<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Path<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Path<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + crate::doc::PathValidate + Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let status = validation_status(&parts.extensions);
        let value = axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| rejection_problem(rejection.status(), rejection.body_text()))?;
        value
            .0
            .validate()
            .map_err(|errors| problem_from_validation(errors, status))?;
        Ok(Self(value.0))
    }
}

/// Query extractor with `Problem`-shaped rejections and constraint validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Query<T>(pub T);

impl<T> Query<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Query<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned + crate::doc::QueryParams + Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let status = validation_status(&parts.extensions);
        let value = axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|rejection| rejection_problem(rejection.status(), rejection.body_text()))?;
        value
            .0
            .validate()
            .map_err(|errors| problem_from_validation(errors, status))?;
        Ok(Self(value.0))
    }
}

/// Form extractor with `Problem`-shaped rejections and constraint validation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Form<T>(pub T);

impl<T> Form<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Form<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Form<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T, S> FromRequest<S> for Form<T>
where
    T: DeserializeOwned + ApiSchema + Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        let skip = req.extensions().get::<SkipValidation>().is_some();
        let status = validation_status(req.extensions());
        let value = axum::Form::<T>::from_request(req, state)
            .await
            .map_err(|rejection| rejection_problem(rejection.status(), rejection.body_text()))?;
        if !skip {
            value
                .0
                .validate()
                .map_err(|errors| problem_from_validation(errors, status))?;
        }
        Ok(Self(value.0))
    }
}

fn problem_from_validation(errors: ValidationErrors, status: StatusCode) -> Problem {
    Problem::from_validation_with_status(errors, status)
}

/// The status a validation failure should use. `400` by default.
pub fn validation_problem(errors: ValidationErrors, status: StatusCode) -> Problem {
    Problem::from_validation_with_status(errors, status)
}
