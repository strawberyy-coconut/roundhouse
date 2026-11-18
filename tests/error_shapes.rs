//! Failure types document their own statuses; `Problem` is one option, not a requirement.

use axum::http::StatusCode;
use roundhouse::doc::{DocumentedError, ResponseDoc, SchemaContext};
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

roundhouse::app_state!(State);

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Thing {
    pub id: u64,
}

#[derive(Debug)]
pub struct DbError;

impl IntoResponse for DbError {
    fn into_response(self) -> Response {
        (StatusCode::NOT_FOUND, "missing").into_response()
    }
}

#[roundhouse::build_only]
impl DocumentedError for DbError {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::problem(404, "Not found")]
    }
}

#[derive(Debug)]
pub enum ApiError {
    NotFound,
    Conflict,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
        };
        (status, "error").into_response()
    }
}

#[roundhouse::build_only]
impl DocumentedError for ApiError {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![
            ResponseDoc::problem(404, "Not found"),
            ResponseDoc::problem(409, "Conflict"),
        ]
    }
}

mod errors {
    use super::*;

    routes! {
        prefix = "/errors",
        "/custom" => { get(custom) },
        "/declared" => { get(declared) },
        "/enum" => { get(enum_error) },
        "/problem" => { get(problem) },
    }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn custom() -> Result<Json<Thing>, DbError> {
        Ok(Json(Thing { id: 1 }))
    }

    #[allow(clippy::result_large_err)]
    #[handler(response(409, DbError))]
    pub async fn declared() -> Result<Json<Thing>, DbError> {
        Ok(Json(Thing { id: 1 }))
    }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn enum_error() -> Result<Json<Thing>, ApiError> {
        Ok(Json(Thing { id: 1 }))
    }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn problem() -> Result<Json<Thing>, Problem> {
        Ok(Json(Thing { id: 1 }))
    }
}

fn app() -> roundhouse::Built {
    roundhouse::App::new(State)
        .include(&errors::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Errors", "0.1.0"))
        .build()
}

#[test]
fn a_custom_error_type_documents_its_own_status() {
    let document = serde_json::to_value(app().document()).unwrap();
    let responses = &document["paths"]["/errors/custom"]["get"]["responses"];
    assert!(responses["200"].is_object());
    assert!(responses["404"].is_object());
    assert!(responses["default"].is_null());
}

#[test]
fn a_declaration_accepts_an_error_only_type() {
    let document = serde_json::to_value(app().document()).unwrap();
    let responses = &document["paths"]["/errors/declared"]["get"]["responses"];
    assert!(responses["200"].is_object());
    assert!(responses["404"].is_object());
    assert!(responses["409"].is_object());
}

#[test]
fn an_error_enum_documents_every_status() {
    let document = serde_json::to_value(app().document()).unwrap();
    let responses = &document["paths"]["/errors/enum"]["get"]["responses"];
    assert!(responses["404"].is_object());
    assert!(responses["409"].is_object());
}

#[test]
fn problem_remains_a_catch_all_helper() {
    let document = serde_json::to_value(app().document()).unwrap();
    let responses = &document["paths"]["/errors/problem"]["get"]["responses"];
    assert!(responses["200"].is_object());
    assert!(responses["default"].is_object());
}
