//! Reusable response wrappers for the common created/no-content cases.

use crate::doc::{ApiResponse, ResponseDoc, SchemaContext, json_response};
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// A `201 Created` JSON response.
///
/// The wrapper separates the status from the body type, so a handler can return
/// `Created<User>` instead of `(StatusCode::CREATED, Json<User>)` while keeping an accurate
/// document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Created<T>(pub T);

impl<T> Created<T> {
    pub fn new(body: T) -> Self {
        Self(body)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: serde::Serialize> IntoResponse for Created<T> {
    fn into_response(self) -> Response {
        (StatusCode::CREATED, Json(self.0)).into_response()
    }
}

impl<T: crate::doc::ApiSchema> ApiResponse for Created<T> {
    const SUCCESS: &'static [u16] = &[201];

    fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<T>(context, 201)
    }
}

/// A `204 No Content` response.
///
/// A 204 has no body, so this wrapper deliberately has no type parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoContent;

impl IntoResponse for NoContent {
    fn into_response(self) -> Response {
        StatusCode::NO_CONTENT.into_response()
    }
}

impl ApiResponse for NoContent {
    const SUCCESS: &'static [u16] = &[204];

    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::new(204, "No Content")]
    }
}
