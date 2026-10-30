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
    serde_json::to_value(app.document()).unwrap()
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

#[derive(Serialize, ToSchema, ApiSchema)]
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

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct OtherPayload {
    pub other: i64,
}

mod override_response {
    use super::*;

    mount!("/override-response");

    routes! {
        get("/x", x, operation_id = "override_x", responses(200 = Payload), override),
    }

    #[handler]
    pub async fn x() -> Json<OtherPayload> {
        Json(OtherPayload { other: 1 })
    }
}

mod conflict_response {
    use super::*;

    mount!("/conflict-response");

    routes! {
        get("/x", x, operation_id = "conflict_x", responses(200 = Payload)),
    }

    #[handler]
    pub async fn x() -> Json<OtherPayload> {
        Json(OtherPayload { other: 1 })
    }
}

mod problem_error {
    use super::*;

    mount!("/problem-error");

    routes! { get("/x", x, operation_id = "problem_error_x") }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn x() -> Result<Json<Payload>, Problem> {
        Ok(Json(Payload { value: 1 }))
    }
}

mod dynamic_status {
    use super::*;

    mount!("/dynamic-status");

    routes! { post("/x", x, operation_id = "dynamic_status_x") }

    #[handler]
    pub async fn x() -> (StatusCode, Json<Payload>) {
        (StatusCode::CREATED, Json(Payload { value: 1 }))
    }
}

#[derive(Debug)]
pub struct BlindError;

impl std::fmt::Display for BlindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("blind")
    }
}

impl std::error::Error for BlindError {}

impl IntoResponse for BlindError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, "blind").into_response()
    }
}

mod blind_error {
    use super::*;

    mount!("/blind-error");

    routes! { get("/x", x, operation_id = "blind_error_x") }

    #[handler]
    pub async fn x() -> Result<Json<Payload>, BlindError> {
        Ok(Json(Payload { value: 1 }))
    }
}

mod extension {
    use super::*;

    mount!("/extension");

    routes! { get("/x", x, operation_id = "extension_x") }

    #[handler]
    pub async fn x(_value: Extension<std::sync::Arc<usize>>) -> String {
        "x".to_string()
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

// Duplicates inside one table are rejected by `routes!` during expansion (see
// tests/ui). These modules keep the aggregate check covered: two separate tables can still
// resolve to the same route, and that is a `try_build` error.

#[derive(Debug)]
pub struct BlindResponse;

impl IntoResponse for BlindResponse {
    fn into_response(self) -> Response {
        "blind".into_response()
    }
}

mod blind_response {
    use super::*;

    mount!("/blind-response");

    routes! { get("/x", x, operation_id = "blind_response_x") }

    #[handler]
    pub async fn x() -> BlindResponse {
        BlindResponse
    }
}

mod cross_duplicate_route_a {
    use super::*;

    mount!("/cross-dup");

    routes! { get("/x", a, operation_id = "cross_dup_a") }

    #[handler]
    pub async fn a() -> &'static str {
        "a"
    }
}

mod cross_duplicate_route_b {
    use super::*;

    mount!("/cross-dup");

    routes! { get("/x", b, operation_id = "cross_dup_b") }

    #[handler]
    pub async fn b() -> &'static str {
        "b"
    }
}

mod cross_duplicate_operation_a {
    use super::*;

    mount!("/cross-op-a");

    routes! { get("/", a, operation_id = "shared_operation") }

    #[handler]
    pub async fn a() -> &'static str {
        "a"
    }
}

mod cross_duplicate_operation_b {
    use super::*;

    mount!("/cross-op-b");

    routes! { get("/", b, operation_id = "shared_operation") }

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
fn a_declared_response_overrides_the_inferred_one() {
    let app = scoped(concat!(module_path!(), "::override_response"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("override builds");
    let document = document(app);
    assert_eq!(
        document["paths"]["/override-response/x"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/Payload"
    );
    assert_eq!(
        document["paths"]["/override-response/x"]["get"]["x-roundhouse-overridden-response"],
        true
    );
}

#[test]
fn a_declared_response_that_contradicts_the_handler_needs_override() {
    let error = scoped(concat!(module_path!(), "::conflict_response"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .err()
        .expect("a silent override must not build");
    assert!(matches!(
        error,
        BuildError::ConflictingResponse { status: 200, .. }
    ));
    assert!(error.to_string().contains("add `override`"));
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
fn problem_error_type_documents_a_default_problem_response() {
    let app = scoped(concat!(module_path!(), "::problem_error"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("Problem builds");
    let document = document(app);
    let responses = &document["paths"]["/problem-error/x"]["get"]["responses"];
    assert_eq!(
        responses["default"]["content"]["application/problem+json"]["schema"]["required"],
        serde_json::json!(["type", "title", "status"])
    );
    assert!(responses["200"].is_object());
}

#[tokio::test]
async fn a_status_bearing_tuple_documents_default_not_200() {
    let app = scoped(concat!(module_path!(), "::dynamic_status"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("dynamic status builds");
    let document = serde_json::to_value(app.document()).unwrap();
    let responses = &document["paths"]["/dynamic-status/x"]["post"]["responses"];
    assert!(responses["default"].is_object());
    assert!(responses["200"].is_null(), "200 would be a lie for 201");

    let response = app
        .oneshot_json(http::Method::POST, "/dynamic-status/x", "{}")
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[test]
fn an_error_type_without_documentation_is_loud() {
    let module = concat!(module_path!(), "::blind_error");
    let app = scoped(module)
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("a blind error still builds by default");
    assert!(
        app.diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("BlindError")),
        "expected a diagnostic naming the undocumented error type"
    );

    let error = scoped(module)
        .strict()
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .err()
        .expect("strict turns the warning into an error");
    assert!(matches!(error, BuildError::Diagnostic { .. }));
}

#[tokio::test]
async fn docs_implies_the_document_route() {
    let app = scoped(concat!(module_path!(), "::declared"))
        .docs()
        .try_build()
        .expect("docs() supplies a document even without openapi()");
    assert_eq!(
        app.oneshot_get("/openapi.json").await.status(),
        StatusCode::OK
    );
    assert_eq!(app.oneshot_get("/docs").await.status(), StatusCode::OK);
}

#[test]
fn a_missing_extension_documents_the_problem_it_produces() {
    let app = scoped(concat!(module_path!(), "::extension"))
        .openapi(|| roundhouse::openapi::Info::new("Honesty", "0.1.0"))
        .try_build()
        .expect("extension route builds");
    let document = serde_json::to_value(app.document()).unwrap();
    assert_eq!(
        document["paths"]["/extension/x"]["get"]["responses"]["500"]["content"]["application/problem+json"]
            ["schema"]["required"],
        serde_json::json!(["type", "title", "status"])
    );
}

#[test]
fn an_unclassifiable_response_still_needs_a_declaration() {
    // The macro cannot see inside a hand-written `IntoResponse`, so the aggregate build remains
    // the place that reports it.
    let error = scoped(concat!(module_path!(), "::blind_response"))
        .try_build()
        .err()
        .expect("a custom response type without a declaration must fail");
    assert!(matches!(error, BuildError::UndeclaredSuccess { .. }));
}

#[test]
fn cross_table_duplicate_routes_fail_the_build() {
    let error = App::new(State)
        .scope(concat!(module_path!(), "::cross_duplicate_route_a"))
        .scope(concat!(module_path!(), "::cross_duplicate_route_b"))
        .try_build()
        .err()
        .expect("two tables resolving to GET /cross-dup/x must fail");
    assert!(matches!(error, BuildError::DuplicateRoute { .. }));
    assert!(error.to_string().contains("/cross-dup/x"));
}

#[test]
fn cross_table_duplicate_operation_ids_fail_the_build() {
    let error = App::new(State)
        .scope(concat!(module_path!(), "::cross_duplicate_operation_a"))
        .scope(concat!(module_path!(), "::cross_duplicate_operation_b"))
        .try_build()
        .err()
        .expect("two tables sharing an operationId must fail");
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
