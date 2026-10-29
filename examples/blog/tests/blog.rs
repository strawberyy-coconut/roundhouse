use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use blog_example::{AppState, app};
use roundhouse::Built;

fn built() -> Built {
    app(AppState::default()).build()
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

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

const NEW_POST: &str =
    r#"{"authorId":1,"title":"Hello world","body":"First post","tags":["rust","axum"]}"#;

#[tokio::test]
async fn nested_mounts_serve_the_full_blog_crud() {
    let app = built();

    assert_eq!(
        send(&app, Method::GET, "/healthz", None).await.status(),
        StatusCode::OK
    );

    let response = send(&app, Method::POST, "/api/blog/posts", Some(NEW_POST)).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::LOCATION], "/api/blog/posts/1");
    let created = body_json(response).await;
    assert_eq!(created["title"], "Hello world");
    assert_eq!(created["status"], "draft");

    let response = send(&app, Method::GET, "/api/blog/posts?tag=rust", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let posts = body_json(response).await;
    assert_eq!(posts.as_array().unwrap().len(), 1);

    let response = send(&app, Method::GET, "/api/blog/authors/1/posts/1", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = send(&app, Method::GET, "/api/blog/authors/2/posts/1", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        &app,
        Method::PUT,
        "/api/blog/posts/1",
        Some(r#"{"title":"Hello again","body":"Updated","tags":["rust"]}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["title"], "Hello again");

    let response = send(
        &app,
        Method::PATCH,
        "/api/blog/posts/1/status",
        Some(r#""published""#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = send(&app, Method::GET, "/api/blog/posts/1", None).await;
    assert_eq!(body_json(response).await["status"], "published");

    let response = send(&app, Method::DELETE, "/api/blog/posts/1", None).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = send(&app, Method::GET, "/api/blog/posts/1", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hidden_and_absolute_routes_behave_differently() {
    let app = built();
    assert_eq!(
        send(&app, Method::GET, "/api/blog/metrics", None)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, Method::GET, "/legacy", None).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        send(&app, Method::GET, "/api/legacy", None).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn document_models_the_nested_tree() {
    let app = built();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();

    assert!(document["paths"]["/api/blog/posts"]["get"].is_object());
    assert!(document["paths"]["/api/blog/posts"]["post"].is_object());
    assert!(document["paths"]["/api/blog/posts/{id}"]["put"].is_object());
    assert!(document["paths"]["/api/blog/posts/{id}/status"]["patch"].is_object());
    assert!(document["paths"]["/legacy"]["get"].is_object());
    assert!(document["paths"]["/api/blog/metrics"].is_null());

    assert_eq!(
        document["paths"]["/api/blog/posts"]["get"]["tags"],
        serde_json::json!(["api", "blog", "posts"])
    );

    let params =
        &document["paths"]["/api/blog/authors/{author_id}/posts/{post_id}"]["get"]["parameters"];
    let names: Vec<&str> = params
        .as_array()
        .unwrap()
        .iter()
        .map(|param| param["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["author_id", "post_id"]);

    assert_eq!(
        document["paths"]["/api/blog/posts"]["post"]["responses"]["201"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/Post"
    );
    assert_eq!(
        document["paths"]["/api/blog/posts"]["post"]["responses"]["201"]["headers"]["Location"]["schema"]
            ["format"],
        "uri"
    );
}

#[tokio::test]
async fn validation_and_not_found_share_the_problem_shape() {
    let app = built();
    let response = send(
        &app,
        Method::POST,
        "/api/blog/posts",
        Some(r#"{"authorId":1,"title":"x","body":"","tags":[]}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    let value = body_json(response).await;
    let errors = value["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error["pointer"] == "/title" && error["code"] == "min_length")
    );
    assert!(
        errors
            .iter()
            .any(|error| error["pointer"] == "/tags" && error["code"] == "min_items")
    );

    let response = send(&app, Method::GET, "/api/blog/posts/99", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let value = body_json(response).await;
    assert_eq!(value["type"], "https://roundhouse.rs/problems/PostNotFound");
    assert_eq!(value["detail"], "post 99 was not found");
}

#[tokio::test]
async fn auth_is_enforced_documented_and_optional_when_asked() {
    let app = built();

    let response = send(&app, Method::GET, "/api/blog/admin/audit", None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");

    let request = Request::builder()
        .uri("/api/blog/admin/audit")
        .header(header::AUTHORIZATION, "Bearer demo")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["secured"], true);

    let response = send(&app, Method::GET, "/api/blog/admin/optional", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let request = Request::builder()
        .uri("/api/blog/admin/optional")
        .header(header::AUTHORIZATION, "Bearer demo")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await;
    assert_eq!(
        http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes(),
        "authenticated"
    );

    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    assert_eq!(
        document["paths"]["/api/blog/admin/audit"]["get"]["security"][0]["bearer"],
        serde_json::json!([])
    );
    assert_eq!(
        document["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
    let optional = document["paths"]["/api/blog/admin/optional"]["get"]["security"]
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
async fn query_and_path_constraints_are_enforced_and_documented() {
    let app = built();

    let response = send(&app, Method::GET, "/api/blog/posts?limit=0", None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = body_json(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/limit");
    assert_eq!(value["errors"][0]["code"], "minimum");

    let response = send(&app, Method::GET, "/api/blog/posts/0", None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = body_json(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/id");
    assert_eq!(value["errors"][0]["code"], "minimum");

    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let params = document["paths"]["/api/blog/posts"]["get"]["parameters"]
        .as_array()
        .unwrap();
    let limit = params
        .iter()
        .find(|param| param["name"] == "limit")
        .unwrap();
    assert_eq!(limit["in"], "query");
    assert_eq!(limit["required"], false);
    assert_eq!(limit["schema"]["minimum"], 1);
    assert_eq!(limit["schema"]["maximum"], 100);

    let path_param = &document["paths"]["/api/blog/posts/{id}"]["get"]["parameters"][0];
    assert_eq!(path_param["name"], "id");
    assert_eq!(path_param["schema"]["minimum"], 1);
}
