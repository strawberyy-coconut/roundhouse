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
    validate_document(app.document().unwrap()).expect("the document validates on its own");
}

#[test]
fn the_validator_rejects_a_capture_without_a_parameter() {
    let app = built();
    let mut value = serde_json::to_value(app.document().unwrap()).unwrap();
    value["paths"]["/validity/items/{id}"]["get"]["parameters"] = serde_json::json!([]);
    let broken: roundhouse::utoipa::openapi::OpenApi = serde_json::from_value(value).unwrap();
    let error = validate_document(&broken).expect_err("a missing parameter is invalid");
    assert!(error.contains("has no path parameter"), "{error}");
}

#[test]
fn the_validator_rejects_a_requirement_without_a_scheme() {
    let app = built();
    let mut value = serde_json::to_value(app.document().unwrap()).unwrap();
    value["paths"]["/validity/items/{id}"]["get"]["security"] =
        serde_json::json!([{ "ghost": [] }]);
    let broken: roundhouse::utoipa::openapi::OpenApi = serde_json::from_value(value).unwrap();
    let error = validate_document(&broken).expect_err("an undeclared scheme is invalid");
    assert!(error.contains("has no scheme"), "{error}");
}
