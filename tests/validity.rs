//! The generated document must satisfy the invariants the document is supposed to guarantee.

use roundhouse::prelude::*;
use roundhouse::{App, validate_document};

#[derive(Clone, Default)]
pub struct State;

mod routes {
    use super::*;

    mount!("/validity");

    routes! { get("/items/{id}", show) }

    #[handler]
    pub async fn show(Path(id): Path<u64>) -> String {
        id.to_string()
    }
}

fn built() -> roundhouse::Built {
    App::new(State)
        .scope(concat!(module_path!(), "::routes"))
        .openapi(|| roundhouse::openapi::Info::new("Validity", "0.1.0"))
        .build()
}

#[test]
fn a_generated_document_is_structurally_valid() {
    let app = built();
    validate_document(app.document()).expect("the document validates on its own");
}

#[test]
fn the_validator_rejects_a_capture_without_a_parameter() {
    let app = built();
    let mut value = serde_json::to_value(app.document()).unwrap();
    value["paths"]["/validity/items/{id}"]["get"]["parameters"] = serde_json::json!([]);
    let broken: roundhouse::utoipa::openapi::OpenApi = serde_json::from_value(value).unwrap();
    let error = validate_document(&broken).expect_err("a missing parameter is invalid");
    assert!(error.contains("has no path parameter"), "{error}");
}

#[test]
fn the_validator_rejects_a_requirement_without_a_scheme() {
    let app = built();
    let mut value = serde_json::to_value(app.document()).unwrap();
    value["paths"]["/validity/items/{id}"]["get"]["security"] =
        serde_json::json!([{ "ghost": [] }]);
    let broken: roundhouse::utoipa::openapi::OpenApi = serde_json::from_value(value).unwrap();
    let error = validate_document(&broken).expect_err("an undeclared scheme is invalid");
    assert!(error.contains("has no scheme"), "{error}");
}

mod invalid_pattern {
    use super::*;

    mount!("/invalid-pattern");

    routes! { post("/", create, operation_id = "invalid_pattern_create") }

    #[derive(serde::Serialize, serde::Deserialize, ApiSchema)]
    pub struct Payload {
        #[schema(pattern = "(")]
        pub value: String,
    }

    #[handler]
    pub async fn create(Json(_body): Json<Payload>) -> axum::http::StatusCode {
        axum::http::StatusCode::NO_CONTENT
    }
}

#[test]
fn an_invalid_schema_pattern_is_a_build_error_not_a_runtime_panic() {
    let error = App::new(State)
        .scope(concat!(module_path!(), "::invalid_pattern"))
        .try_build()
        .err()
        .expect("an invalid pattern must fail the build");
    assert!(matches!(error, roundhouse::BuildError::InvalidDocument { .. }));
    assert!(error.to_string().contains("not a valid regular expression"));
}
