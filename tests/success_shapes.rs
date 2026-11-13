//! Return-type inference publishes the status the runtime actually produces.

use axum::http::StatusCode;
use roundhouse::prelude::*;
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Thing {
    pub id: u64,
}

/// A response body that is serializable but publishes no schema.
#[derive(Serialize)]
pub struct Plain {
    pub value: String,
}

mod shapes {
    use super::*;
    use axum::http::HeaderMap;

    routes! {
        prefix = "/shapes",

        "/unit" => { get(unit, operation_id = "shape_unit") },
        "/explicit-unit" => { get(explicit_unit, operation_id = "shape_explicit_unit") },
        "/json" => { get(json, operation_id = "shape_json") },
        "/text" => { get(text, operation_id = "shape_text") },
        "/status" => { get(status, operation_id = "shape_status") },
        "/dynamic" => { get(dynamic, operation_id = "shape_dynamic") },
        "/created" => { get(created, operation_id = "shape_created") },
        "/no-content" => { get(no_content, operation_id = "shape_no_content") },
        "/created-plain" => { get(created_plain, operation_id = "shape_created_plain") },
        "/result-created-plain" => {
            get(result_created_plain, operation_id = "shape_result_created_plain")
        },
        "/redirect" => { get(redirect, operation_id = "shape_redirect") },
    }

    #[derive(ApiResponse)]
    pub enum Redirect {
        #[status(304)]
        NotModified,
    }

    #[handler]
    pub async fn redirect() -> Redirect {
        Redirect::NotModified
    }

    #[handler]
    pub async fn unit() {}

    #[handler]
    #[allow(clippy::unused_unit)]
    pub async fn explicit_unit() -> () {}

    #[handler]
    pub async fn json() -> Json<Thing> {
        Json(Thing { id: 1 })
    }

    #[handler]
    pub async fn text() -> &'static str {
        "ok"
    }

    #[handler]
    pub async fn status() -> StatusCode {
        StatusCode::NO_CONTENT
    }

    #[handler]
    pub async fn dynamic() -> (StatusCode, Json<Thing>) {
        (StatusCode::CREATED, Json(Thing { id: 1 }))
    }

    #[handler]
    pub async fn created() -> Created<Thing> {
        Created::new(Thing { id: 1 })
    }

    #[handler]
    pub async fn no_content() -> NoContent {
        NoContent
    }

    #[handler]
    pub async fn created_plain() -> Created<Plain> {
        Created::new(Plain {
            value: "plain".to_string(),
        })
    }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn result_created_plain() -> Result<Created<Plain>, Problem> {
        Ok(Created::new(Plain {
            value: "plain".to_string(),
        }))
    }

    /// Compiles as `IntoResponse`, but roundhouse has no `ApiResponse` impl for it, so the build
    /// must require a declaration instead of guessing a status.
    pub mod parts_tuple {
        use super::*;

        routes! {
            prefix = "/parts-tuple",
            "/x" => { get(x, operation_id = "shape_parts_tuple_x") },
        }

        #[handler]
        pub async fn x() -> (HeaderMap, Json<Thing>) {
            (HeaderMap::new(), Json(Thing { id: 1 }))
        }
    }
}

fn app() -> roundhouse::Built {
    roundhouse::App::new(State)
        .include(&shapes::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Shapes", "0.1.0"))
        .permissive()
        .build()
}

#[test]
fn redirect_statuses_count_as_success() {
    assert_eq!(shapes::Redirect::SUCCESS, &[304]);
    let document = serde_json::to_value(app().document()).unwrap();
    assert!(document["paths"]["/shapes/redirect"]["get"]["responses"]["304"].is_object());
}

#[test]
fn return_shapes_document_the_runtime_status() {
    let document = serde_json::to_value(app().document()).unwrap();
    let paths = &document["paths"];

    assert!(paths["/shapes/unit"]["get"]["responses"]["200"].is_object());
    assert!(paths["/shapes/explicit-unit"]["get"]["responses"]["200"].is_object());
    assert!(paths["/shapes/json"]["get"]["responses"]["200"].is_object());
    assert!(paths["/shapes/text"]["get"]["responses"]["200"].is_object());
    assert!(paths["/shapes/status"]["get"]["responses"]["default"].is_object());
    assert!(paths["/shapes/dynamic"]["get"]["responses"]["default"].is_object());
    assert!(paths["/shapes/created"]["get"]["responses"]["201"].is_object());
    assert!(paths["/shapes/no-content"]["get"]["responses"]["204"].is_object());
    assert!(paths["/shapes/created-plain"]["get"]["responses"]["201"].is_object());
    assert!(paths["/shapes/result-created-plain"]["get"]["responses"]["201"].is_object());
}

#[tokio::test]
async fn a_no_return_handler_answers_200() {
    let built = app();
    let response = built.oneshot_get("/shapes/unit").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn an_explicit_unit_handler_answers_200() {
    let built = app();
    let response = built.oneshot_get("/shapes/explicit-unit").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn a_known_shape_without_a_probe_is_marked_unverified() {
    let document = serde_json::to_value(app().document()).unwrap();
    assert_eq!(
        document["paths"]["/shapes/created-plain"]["get"]["x-roundhouse-unverified-response"],
        true
    );
    assert_eq!(
        document["paths"]["/shapes/result-created-plain"]["get"]["x-roundhouse-unverified-response"],
        true
    );
}

#[test]
fn strict_rejects_the_unverified_fallback() {
    let error = roundhouse::App::new(State)
        .include(&shapes::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Shapes", "0.1.0"))
        .strict()
        .try_build()
        .err()
        .expect("strict must reject an unverified response");
    assert!(matches!(
        error,
        roundhouse::BuildError::UnverifiedResponse { .. }
    ));
}

#[test]
fn a_tuple_with_response_parts_still_needs_a_declaration() {
    let error = roundhouse::App::new(State)
        .include(&shapes::parts_tuple::MODULE)
        .try_build()
        .err()
        .expect("an undocumented tuple response must fail the build");
    assert!(matches!(
        error,
        roundhouse::BuildError::UndeclaredSuccess { .. }
    ));
}

roundhouse::app_state!(State);
