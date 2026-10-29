//! Build-time honesty: fallible builds, loud derivation, and declared metadata.

use axum::http::StatusCode;
use roundhouse::prelude::*;
use roundhouse::{App, BuildError, Built};
use serde::Serialize;

#[derive(Clone)]
pub struct State;

fn scoped(module: &'static str) -> App<State> {
    App::new(State).scope(module)
}

fn document(app: Built) -> serde_json::Value {
    serde_json::to_value(app.document().expect("openapi enabled")).unwrap()
}

mod undeclared {
    use super::*;

    mount!("/undeclared");

    routes! {
        get("/x", x, operation_id = "undeclared_x"),
    }

    #[handler]
    pub async fn x() -> impl IntoResponse {
        "x"
    }
}

mod declared {
    use super::*;

    mount!("/declared");

    routes! {
        get("/x", x, operation_id = "declared_x", responses(200)),
    }

    #[handler]
    pub async fn x() -> impl IntoResponse {
        "x"
    }
}

#[derive(Serialize, ApiSchema)]
pub struct Payload {
    pub value: i64,
}

mod typed {
    use super::*;

    mount!("/typed");

    routes! {
        get("/x", x, operation_id = "typed_x", responses(200 = Payload)),
    }

    #[handler]
    pub async fn x() -> impl IntoResponse {
        Json(Payload { value: 1 })
    }
}

mod foreign {
    use super::*;

    mount!("/foreign");

    #[derive(Serialize, serde::Deserialize)]
    pub struct Plain {
        pub id: i64,
    }

    routes! {
        get("/response", response, operation_id = "foreign_response"),
        post("/request", request, operation_id = "foreign_request"),
    }

    #[handler]
    pub async fn response() -> axum::Json<Plain> {
        axum::Json(Plain { id: 1 })
    }

    #[handler]
    pub async fn request(body: axum::Json<Plain>) -> StatusCode {
        let _ = body;
        StatusCode::NO_CONTENT
    }
}

mod duplicate_route {
    use super::*;

    mount!("/duplicate-route");

    routes! {
        get("/x", a, operation_id = "duplicate_route_a"),
        get("/x", b, operation_id = "duplicate_route_b"),
    }

    #[handler]
    pub async fn a() -> &'static str {
        "a"
    }

    #[handler]
    pub async fn b() -> &'static str {
        "b"
    }
}

mod normalised_route {
    use super::*;

    mount!("/normalised");

    routes! {
        get("/x/{id}", a, operation_id = "normalised_a"),
        get("/x/{post_id}", b, operation_id = "normalised_b"),
    }

    #[handler]
    pub async fn a() -> &'static str {
        "a"
    }

    #[handler]
    pub async fn b() -> &'static str {
        "b"
    }
}

mod duplicate_operation {
    use super::*;

    mount!("/duplicate-operation");

    routes! {
        get("/a", a, operation_id = "same"),
        get("/b", b, operation_id = "same"),
    }

    #[handler]
    pub async fn a() -> &'static str {
        "a"
    }

    #[handler]
    pub async fn b() -> &'static str {
        "b"
    }
}

mod metadata {
    use super::*;

    mount!("/metadata");

    routes! {
        get("/x", x, operation_id = "metadata_x", deprecated, public),
    }

    #[handler]
    pub async fn x() -> Json<Payload> {
        Json(Payload { value: 1 })
    }
}

mod default_status {
    use super::*;

    mount!("/default-status");

    routes! {
        get("/x", x, operation_id = "default_status_x"),
    }

    #[handler]
    pub async fn x() -> StatusCode {
        StatusCode::NO_CONTENT
    }
}

#[test]
fn undeclared_success_is_a_build_error() {
    let error = scoped(concat!(module_path!(), "::undeclared"))
        .try_build()
        .err()
        .expect("impl IntoResponse without responses(..) must fail");
    assert!(matches!(error, BuildError::UndeclaredSuccess { .. }));
    assert!(error.to_string().contains("declares no success response"));
}

#[test]
fn declared_success_builds_and_documents_no_body() {
    let app = scoped(concat!(module_path!(), "::declared"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("declared success builds");
    let document = document(app);
    assert!(document["paths"]["/declared/x"]["get"]["responses"]["200"].is_object());
}

#[test]
fn a_typed_response_declaration_uses_the_named_schema() {
    let app = scoped(concat!(module_path!(), "::typed"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("typed response builds");
    let document = document(app);
    assert_eq!(
        document["paths"]["/typed/x"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/Payload"
    );
}

#[test]
fn foreign_extractors_are_marked_and_strict_turns_the_marker_into_an_error() {
    let module = concat!(module_path!(), "::foreign");
    let app = scoped(module)
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("foreign extractors still build by default");
    let document = document(app);
    assert_eq!(
        document["paths"]["/foreign/request"]["post"]["x-roundhouse-unverified-body"],
        true
    );

    let error = scoped(module)
        .strict()
        .try_build()
        .err()
        .expect("strict mode rejects unverified body extractors");
    assert!(matches!(error, BuildError::UnverifiedBody { .. }));
}

#[test]
fn duplicate_routes_fail_the_build() {
    let error = scoped(concat!(module_path!(), "::duplicate_route"))
        .try_build()
        .err()
        .expect("two GET /x entries must fail");
    assert!(matches!(error, BuildError::DuplicateRoute { .. }));
}

#[test]
fn templates_are_normalised_before_duplicate_comparison() {
    let error = scoped(concat!(module_path!(), "::normalised_route"))
        .try_build()
        .err()
        .expect("{id} and {post_id} are the same route shape");
    assert!(matches!(error, BuildError::DuplicateRoute { .. }));
}

#[test]
fn duplicate_operation_ids_fail_the_build() {
    let error = scoped(concat!(module_path!(), "::duplicate_operation"))
        .try_build()
        .err()
        .expect("two operations cannot share an operationId");
    assert!(matches!(error, BuildError::DuplicateOperationId { .. }));
    assert!(error.to_string().contains("operation_id"));
}

#[test]
fn deprecated_and_public_are_no_longer_no_ops() {
    let app = scoped(concat!(module_path!(), "::metadata"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("metadata route builds");
    let document = document(app);
    let operation = &document["paths"]["/metadata/x"]["get"];
    assert_eq!(operation["deprecated"], true);
    assert_eq!(operation["x-roundhouse-public"], true);
}

#[test]
fn a_status_code_return_documents_a_default_response() {
    let app = scoped(concat!(module_path!(), "::default_status"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("StatusCode is a declared shape");
    let document = document(app);
    assert!(document["paths"]["/default-status/x"]["get"]["responses"]["default"].is_object());
}
