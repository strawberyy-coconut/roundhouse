use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use layers_example::{AppState, app, embedded};
use roundhouse::Built;

fn built() -> Built {
    app(AppState).build()
}

async fn send(router: axum::Router, method: Method, uri: &str) -> axum::response::Response {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    tower::ServiceExt::oneshot(router, request).await.unwrap()
}

#[tokio::test]
async fn every_layer_level_adds_its_header() {
    let app = built();

    let response = send(app.router(), Method::GET, "/full/ok").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-global"], "global");
    assert_eq!(response.headers()["x-module"], "module");

    let response = send(app.router(), Method::GET, "/route-layer/ok").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-global"], "global");
    assert_eq!(response.headers()["x-route-layer"], "route-layer");

    let response = send(app.router(), Method::GET, "/entry/ok").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-global"], "global");
    assert_eq!(response.headers()["x-entry"], "entry");
}

#[tokio::test]
async fn fallback_and_merge_are_wrapped_by_the_global_layer() {
    let app = built();

    let response = send(app.router(), Method::GET, "/missing").await;
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(response.headers()["x-global"], "global");
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"custom fallback");

    let response = send(app.router(), Method::GET, "/merged").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-global"], "global");
}

#[tokio::test]
async fn app_router_can_be_embedded_in_a_larger_app() {
    let router = embedded(AppState);

    let response = send(router.clone(), Method::GET, "/outer").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("x-global").is_none());

    let response = send(router.clone(), Method::GET, "/embedded/full/ok").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-global"], "global");
    assert_eq!(response.headers()["x-module"], "module");

    let response = send(router, Method::GET, "/embedded/missing").await;
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(response.headers()["x-global"], "global");
}

#[tokio::test]
async fn route_layer_matches_routes_without_wrapping_outer_404s() {
    let app = built();
    let response = send(app.router(), Method::GET, "/route-layer/missing").await;
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(response.headers()["x-global"], "global");
    assert!(response.headers().get("x-route-layer").is_none());
}
