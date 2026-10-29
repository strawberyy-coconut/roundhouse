use auth_example::{AppState, app};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use roundhouse::Built;

fn built() -> Built {
    app(AppState::default()).build()
}

async fn request(app: &Built, uri: &str, headers: &[(&str, &str)]) -> axum::response::Response {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.oneshot(builder.body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn bearer_auth_is_enforced_with_www_authenticate() {
    let app = built();

    let response = request(&app, "/private", &[]).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");

    let response = request(
        &app,
        "/private",
        &[(header::AUTHORIZATION.as_str(), "Bearer reader")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn scopes_and_optional_auth_are_documented() {
    let app = built();

    assert_eq!(
        request(
            &app,
            "/scoped",
            &[(header::AUTHORIZATION.as_str(), "Bearer reader")]
        )
        .await
        .status(),
        StatusCode::OK
    );
    let response = request(&app, "/optional", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], br#"{"message":"anonymous"}"#);

    let response = request(
        &app,
        "/optional",
        &[(header::AUTHORIZATION.as_str(), "Bearer reader")],
    )
    .await;
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], br#"{"message":"authenticated"}"#);

    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    assert_eq!(
        document["paths"]["/scoped"]["get"]["security"][0]["bearer"],
        serde_json::json!(["read:posts"])
    );
    let optional = document["paths"]["/optional"]["get"]["security"]
        .as_array()
        .unwrap();
    assert_eq!(optional.len(), 2);
    assert!(
        optional
            .iter()
            .any(|requirement| requirement.as_object().unwrap().is_empty())
    );
}

#[tokio::test]
async fn two_required_extractors_and_into_one_requirement() {
    let app = built();

    let response = request(
        &app,
        "/admin",
        &[(header::AUTHORIZATION.as_str(), "Bearer admin")],
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = request(
        &app,
        "/admin",
        &[
            (header::AUTHORIZATION.as_str(), "Bearer reader"),
            ("X-API-Key", "secret"),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = request(
        &app,
        "/admin",
        &[
            (header::AUTHORIZATION.as_str(), "Bearer admin"),
            ("X-API-Key", "secret"),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let security = document["paths"]["/admin"]["get"]["security"]
        .as_array()
        .unwrap();
    assert_eq!(security.len(), 1);
    assert!(security[0]["bearer"].is_array());
    assert!(security[0]["api-key"].is_array());
    assert_eq!(
        document["components"]["securitySchemes"]["api-key"]["name"],
        "X-API-Key"
    );
    assert_eq!(
        document["components"]["securitySchemes"]["api-key"]["in"],
        "header"
    );
    assert_eq!(
        document["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
}

#[tokio::test]
async fn auth_failures_are_documented_and_the_ui_is_served() {
    let app = built();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    assert!(document["paths"]["/private"]["get"]["responses"]["401"].is_object());
    assert!(document["paths"]["/private"]["get"]["responses"]["403"].is_object());
    assert_eq!(
        document["paths"]["/private"]["get"]["security"][0]["bearer"],
        serde_json::json!([])
    );

    let response = app.oneshot_get("/docs").await;
    assert_eq!(response.status(), StatusCode::OK);
}
