//! `App::swagger()` serves a self-hosted Swagger UI without the CDN.

use axum::http::{StatusCode, header};
use http_body_util::BodyExt;
use roundhouse::App;
use roundhouse::prelude::*;
use roundhouse::utoipa::openapi::Info;

#[derive(Clone, Default)]
pub struct State;

mod routes {
    use super::*;

    routes! {
        prefix = "/v1",
        "/hello" => { get(hello) },
    }

    #[handler]
    pub async fn hello() -> &'static str {
        "hello"
    }
}

fn app() -> App<State> {
    roundhouse::app!(State, routes)
        .openapi(|| Info::new("Swagger", "0.1.0"))
        .swagger()
}

async fn body_string(response: axum::response::Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn swagger_ui_and_assets_are_served_locally() {
    let app = app().build();

    let response = app.oneshot_get("/docs/swagger").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let html = body_string(response).await;
    assert!(html.contains("/docs/swagger/assets/swagger-ui.css"));
    assert!(html.contains("/docs/swagger/assets/swagger-ui-bundle.js"));
    assert!(html.contains("/docs/swagger/assets/oauth2-redirect.html"));
    assert!(!html.contains("unpkg"));

    for (path, content_type) in [
        (
            "/docs/swagger/assets/swagger-ui.css",
            "text/css; charset=utf-8",
        ),
        (
            "/docs/swagger/assets/swagger-ui-bundle.js",
            "text/javascript; charset=utf-8",
        ),
        (
            "/docs/swagger/assets/oauth2-redirect.html",
            "text/html; charset=utf-8",
        ),
        ("/docs/swagger/assets/favicon-32x32.png", "image/png"),
    ] {
        let response = app.oneshot_get(path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            content_type,
            "{path}"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!bytes.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn swagger_ui_can_be_moved() {
    let app = roundhouse::app!(State, routes)
        .openapi(|| Info::new("Swagger", "0.1.0"))
        .swagger_route("/reference")
        .build();

    let response = app.oneshot_get("/reference").await;
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_string(response).await;
    assert!(html.contains("/reference/assets/swagger-ui.css"));
    assert!(!html.contains("/docs/swagger"));

    let response = app.oneshot_get("/reference/assets/swagger-ui.css").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn swagger_implies_the_document_route() {
    let app = roundhouse::app!(State, routes)
        .swagger()
        .try_build()
        .expect("swagger() supplies a document even without openapi()");
    assert_eq!(
        app.oneshot_get("/openapi.json").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot_get("/docs/swagger").await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn docs_and_swagger_can_be_served_together() {
    let app = roundhouse::app!(State, routes)
        .openapi(|| Info::new("Swagger", "0.1.0"))
        .docs()
        .swagger()
        .build();

    let response = app.oneshot_get("/docs").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_string(response).await.contains("unpkg.com"));

    let response = app.oneshot_get("/docs/swagger").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!body_string(response).await.contains("unpkg.com"));
}

mod collision {
    use super::*;

    routes! {
        prefix = "/docs/swagger/assets",
        "/swagger-ui.css" => { get(swagger_ui_css, operation_id = "collision_swagger_ui_css") },
    }

    #[handler]
    pub async fn swagger_ui_css() -> &'static str {
        "not swagger"
    }
}

#[test]
fn a_declared_route_cannot_claim_a_swagger_asset_path() {
    let error = App::new(State)
        .include(&collision::MODULE)
        .swagger()
        .try_build()
        .err()
        .expect("the Swagger asset route must be reserved");
    assert!(matches!(
        error,
        roundhouse::BuildError::RouteConflict { .. }
    ));
    assert!(error.to_string().contains("Swagger UI asset"));
}
