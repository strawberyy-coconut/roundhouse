//! Extractor documentation is resolved from the type, not the identifier.
//!
//! A parameter spelled `Json<T>` may resolve to `roundhouse::Json<T>`, `axum::Json<T>`, an alias
//! of either, or an unrelated type with the same name. The document must describe the extractor
//! that actually runs.

use axum::http::StatusCode;
use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct State;

fn app(module: &'static str) -> roundhouse::Built {
    roundhouse::App::new(State)
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Extractors", "0.1.0"))
        .build()
}

#[derive(Debug, Serialize, Deserialize, ToSchema, ApiSchema)]
pub struct Contact {
    #[schema(min_length = 3)]
    pub title: String,
}

mod axum_alias {
    use super::*;
    // The explicit import wins over the prelude's `Json`.
    use axum::Json;

    mount!("/axum-alias");

    routes! { post("/", create, operation_id = "axum_alias_create") }

    #[handler]
    pub async fn create(Json(_body): Json<Contact>) -> StatusCode {
        StatusCode::CREATED
    }
}

mod axum_alias_unvalidated {
    use super::*;
    use axum::Json;

    mount!("/axum-alias-unvalidated");

    routes! {
        post("/", create, operation_id = "axum_alias_unvalidated_create", validate = false)
    }

    #[handler]
    pub async fn create(Json(_body): Json<Contact>) -> StatusCode {
        StatusCode::CREATED
    }
}

mod roundhouse_alias {
    use super::*;
    use roundhouse::Json as Payload;

    mount!("/roundhouse-alias");

    routes! { post("/", create, operation_id = "roundhouse_alias_create") }

    #[handler]
    pub async fn create(Payload(_body): Payload<Contact>) -> StatusCode {
        StatusCode::CREATED
    }
}

mod body_alias {
    use super::*;
    // Named `Body`, but it is axum's JSON extractor: it must not document `octet-stream`.
    use axum::Json as Body;

    mount!("/body-alias");

    routes! { post("/", create, operation_id = "body_alias_create") }

    #[handler]
    pub async fn create(Body(_body): Body<Contact>) -> StatusCode {
        StatusCode::CREATED
    }
}

#[test]
fn axum_json_is_validated_and_documented() {
    let built = app(concat!(module_path!(), "::axum_alias"));
    let document = serde_json::to_value(built.document()).unwrap();
    let operation = &document["paths"]["/axum-alias"]["post"];

    assert!(
        operation.get("x-roundhouse-unverified-body").is_none(),
        "the route's shim validated the body, so it is verified: {operation}"
    );
    assert_eq!(
        operation["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/Contact"
    );
    assert_eq!(
        document["components"]["schemas"]["Contact"]["properties"]["title"]["minLength"],
        3
    );
}

#[tokio::test]
async fn axum_json_is_validated_by_default() {
    let built = app(concat!(module_path!(), "::axum_alias"));
    // The route entry validates by default, so the shim enforces the constraint even though the
    // handler uses axum's extractor.
    let response = built
        .oneshot_json(http::Method::POST, "/axum-alias", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn validate_false_opts_a_route_out() {
    let built = app(concat!(module_path!(), "::axum_alias_unvalidated"));
    let response = built
        .oneshot_json(
            http::Method::POST,
            "/axum-alias-unvalidated",
            r#"{"title":"ab"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[test]
fn an_alias_of_roundhouse_json_still_documents_and_enforces_the_constraint() {
    let built = app(concat!(module_path!(), "::roundhouse_alias"));
    let document = serde_json::to_value(built.document()).unwrap();
    let schema = &document["paths"]["/roundhouse-alias"]["post"]["requestBody"]["content"]["application/json"]
        ["schema"];
    assert_eq!(schema["$ref"], "#/components/schemas/Contact");
    assert_eq!(
        document["components"]["schemas"]["Contact"]["properties"]["title"]["minLength"],
        3
    );
}

#[tokio::test]
async fn an_alias_of_roundhouse_json_still_validates_at_runtime() {
    let built = app(concat!(module_path!(), "::roundhouse_alias"));
    let response = built
        .oneshot_json(http::Method::POST, "/roundhouse-alias", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn a_json_extractor_aliased_as_body_is_not_documented_as_octet_stream() {
    let built = app(concat!(module_path!(), "::body_alias"));
    let document = serde_json::to_value(built.document()).unwrap();
    let operation = &document["paths"]["/body-alias"]["post"];

    assert!(
        operation.get("x-roundhouse-unverified-body").is_none(),
        "the route's shim validated the body, so it is verified: {operation}"
    );
    let body = operation["requestBody"].to_string();
    assert!(
        !body.contains("application/octet-stream"),
        "the resolved type is JSON, not a raw body: {body}"
    );
}
