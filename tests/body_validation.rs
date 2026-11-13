//! Route validation policy: whole-route opt-out, body-only opt-out and acknowledgement.

#![cfg(feature = "schema-engine")]

use axum::http::StatusCode;
use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct State;

#[derive(Debug, Deserialize, Serialize, ToSchema, ApiSchema)]
pub struct Payload {
    #[schema(min_length = 3)]
    pub name: String,
}

#[derive(Debug, Deserialize, QueryParams)]
pub struct Filter {
    #[schema(min_length = 3)]
    pub q: String,
}

mod body_opted_out {
    use super::*;

    routes! {
        prefix = "/body",
        "/plain" => { post(plain, validate_body = false) },
        "/strict" => { post(strict_body, validate_body = false) },
    }

    #[handler]
    pub async fn plain(Query(_): Query<Filter>, Json(payload): Json<Payload>) -> String {
        payload.name
    }

    #[handler]
    pub async fn strict_body(
        Query(_): Query<Filter>,
        Strict(Json(payload)): Strict<Json<Payload>>,
    ) -> String {
        payload.name
    }
}

mod whole_route_opted_out {
    use super::*;

    routes! {
        prefix = "/whole",
        "/quiet" => {
            post(quiet, validate = false, unverified = "opaque import payload"),
        },
    }

    #[handler]
    pub async fn quiet(Query(_): Query<Filter>, Json(payload): Json<Payload>) -> String {
        payload.name
    }
}

mod unacknowledged {
    use super::*;

    routes! {
        prefix = "/loud",
        "/x" => { post(x, validate = false) },
    }

    #[handler]
    pub async fn x(Json(payload): Json<Payload>) -> String {
        payload.name
    }
}

fn app(module: &'static roundhouse::Module) -> roundhouse::Built {
    roundhouse::App::new(State)
        .include(module)
        .openapi(|| roundhouse::openapi::Info::new("Policy", "0.1.0"))
        .build()
}

#[tokio::test]
async fn body_only_opt_out_still_validates_parts() {
    let built = app(&body_opted_out::MODULE);

    // The body violates `min_length`, but the body policy is off.
    let response = built
        .oneshot_json(
            axum::http::Method::POST,
            "/body/plain?q=long-enough",
            r#"{"name":"x"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    // The query part still runs the schema.
    let response = built
        .oneshot_json(
            axum::http::Method::POST,
            "/body/plain?q=x",
            r#"{"name":"fine"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Malformed JSON is still rejected during extraction.
    let response = built
        .oneshot_json(
            axum::http::Method::POST,
            "/body/plain?q=long-enough",
            r#"{"name":"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn strict_json_honours_the_body_policy() {
    let built = app(&body_opted_out::MODULE);
    let response = built
        .oneshot_json(
            axum::http::Method::POST,
            "/body/strict?q=long-enough",
            r#"{"name":"x"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn the_document_records_the_body_opt_out() {
    let built = app(&body_opted_out::MODULE);
    let document = serde_json::to_value(built.document()).unwrap();
    let operation = &document["paths"]["/body/plain"]["post"];
    assert_eq!(operation["x-roundhouse-body-validation"], false);
    assert_eq!(operation["x-roundhouse-unverified-body"], true);
}

#[test]
fn an_acknowledged_whole_route_opt_out_builds_strict() {
    roundhouse::App::new(State)
        .include(&whole_route_opted_out::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Policy", "0.1.0"))
        .strict()
        .try_build()
        .expect("an acknowledged opt-out must pass strict");
}

#[test]
fn an_unacknowledged_whole_route_opt_out_fails_strict() {
    let error = roundhouse::App::new(State)
        .include(&unacknowledged::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Policy", "0.1.0"))
        .strict()
        .try_build()
        .err()
        .expect("an unacknowledged opt-out must fail strict");
    assert!(matches!(
        error,
        roundhouse::BuildError::UnverifiedBody { .. }
    ));
}

roundhouse::app_state!(State);
