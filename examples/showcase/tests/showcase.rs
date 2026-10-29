use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use roundhouse::Built;
use showcase_example::{AppState, app};

fn built() -> Built {
    app(AppState).build()
}

async fn send(
    app: &Built,
    method: Method,
    uri: &str,
    body: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let request = builder
        .body(Body::from(body.unwrap_or_default().to_string()))
        .unwrap();
    app.oneshot(request).await
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn enum_variants_validate_with_the_configured_status() {
    let app = built();
    let response = send(
        &app,
        Method::POST,
        "/events",
        Some(r#"{"kind":"created","title":"x"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let value = json_body(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/title");
    assert_eq!(value["errors"][0]["code"], "min_length");

    let response = send(
        &app,
        Method::POST,
        "/events",
        Some(r#"{"kind":"created","title":"valid title"}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn validate_with_and_utoipa_interop_work() {
    let app = built();

    let response = send(&app, Method::POST, "/even", Some(r#"{"value":3}"#)).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let value = json_body(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/value");
    assert_eq!(value["errors"][0]["code"], "even");

    let response = send(&app, Method::POST, "/even", Some(r#"{"value":4}"#)).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = app.oneshot_get("/utoipa").await;
    assert_eq!(response.status(), StatusCode::OK);
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let schema = &document["paths"]["/utoipa"]["get"]["responses"]["200"]["content"]["application/json"]
        ["schema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["name"]["type"], "string");
}

#[tokio::test]
async fn collisions_hidden_any_erased_and_cfg_are_all_served() {
    let app = built();

    assert_eq!(
        send(&app, Method::GET, "/a/thing", None).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, Method::GET, "/b/thing", None).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, Method::GET, "/internal/health", None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, Method::GET, "/echo", None).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, Method::POST, "/echo", None).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, Method::GET, "/extras", None).await.status(),
        StatusCode::OK
    );

    let response = app.oneshot_get("/erased").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"42");
}

#[tokio::test]
async fn document_reflects_collisions_hidden_routes_and_rejection_policy() {
    let app = built();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();

    assert!(
        document["components"]["schemas"]["showcase_example.routes.collisions.a.Thing"].is_object()
    );
    assert!(
        document["components"]["schemas"]["showcase_example.routes.collisions.b.Thing"].is_object()
    );
    assert!(document["paths"]["/internal/health"].is_null());
    assert!(document["paths"]["/echo"].is_null());

    let responses = &document["paths"]["/events"]["post"]["responses"];
    assert!(responses.get("400").is_none());
    assert!(responses.get("415").is_none());

    assert_eq!(
        document["paths"]["/events"]["post"]["tags"],
        serde_json::json!(["showcase"])
    );
}

#[tokio::test]
async fn docs_ui_is_served_and_response_validation_stays_green() {
    let app = built();
    let response = app.oneshot_get("/docs").await;
    assert_eq!(response.status(), StatusCode::OK);

    // validate_responses() is enabled on this app; a mismatch would become a 500.
    for uri in [
        "/a/thing",
        "/b/thing",
        "/internal/health",
        "/utoipa",
        "/erased",
    ] {
        assert_eq!(app.oneshot_get(uri).await.status(), StatusCode::OK, "{uri}");
    }
}
