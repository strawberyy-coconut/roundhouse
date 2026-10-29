use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use roundhouse::Built;
use uploads_example::{AppState, app};

fn built() -> Built {
    app(AppState).build()
}

fn multipart(boundary: &str, parts: &[(&str, Option<&str>, Option<&str>, &str)]) -> String {
    let mut body = String::new();
    for (name, filename, content_type, value) in parts {
        body.push_str(&format!("--{boundary}\r\n"));
        match filename {
            Some(filename) => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n"
            )),
            None => body.push_str(&format!(
                "Content-Disposition: form-data; name=\"{name}\"\r\n"
            )),
        }
        if let Some(content_type) = content_type {
            body.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        body.push_str("\r\n");
        body.push_str(value);
        body.push_str("\r\n");
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    body
}

async fn upload(app: &Built, body: &str) -> axum::response::Response {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/uploads")
        .header(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=uploads-boundary",
        )
        .body(Body::from(body.to_string()))
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
async fn multipart_files_and_temp_files_are_extracted() {
    let app = built();
    let body = multipart(
        "uploads-boundary",
        &[
            ("file", Some("note.txt"), Some("text/plain"), "hello"),
            (
                "preview",
                Some("preview.md"),
                Some("text/markdown"),
                "# preview",
            ),
            (
                "temp",
                Some("temp.bin"),
                Some("application/octet-stream"),
                "temporary",
            ),
            ("tags", None, None, "rust"),
            ("tags", None, None, "files"),
            ("title", None, None, "A demo"),
        ],
    );
    let response = upload(&app, &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value = json_body(response).await;
    assert_eq!(value["file_name"], "note.txt");
    assert_eq!(value["file_len"], 5);
    assert_eq!(value["preview_len"], 9);
    assert_eq!(value["temp_len"], 9);
    assert_eq!(value["tags"], serde_json::json!(["rust", "files"]));
    assert_eq!(value["title"], "A demo");
}

#[tokio::test]
async fn multipart_limits_are_enforced() {
    let app = built();

    let body = multipart("uploads-boundary", &[("tags", None, None, "rust")]);
    let response = upload(&app, &body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = json_body(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/file");

    let body = multipart(
        "uploads-boundary",
        &[
            ("file", Some("note.txt"), Some("text/plain"), "hello"),
            (
                "preview",
                Some("preview.json"),
                Some("application/json"),
                "{}",
            ),
        ],
    );
    let response = upload(&app, &body).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn downloads_and_sse_set_their_documented_headers() {
    let app = built();

    let response = app.oneshot_get("/uploads/download").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"hello.txt\""
    );

    let response = app.oneshot_get("/uploads/raw").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );

    let response = app.oneshot_get("/uploads/events").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
}

#[tokio::test]
async fn custom_openapi_and_docs_routes_are_served() {
    let app = built();

    let response = app.oneshot_get("/spec.json").await;
    assert_eq!(response.status(), StatusCode::OK);
    let document = json_body(response).await;
    assert_eq!(document["openapi"], "3.1.0");

    let request = &document["paths"]["/uploads"]["post"]["requestBody"]["content"]["multipart/form-data"]
        ["schema"];
    assert_eq!(request["properties"]["file"]["format"], "binary");
    assert_eq!(request["properties"]["preview"]["x-max-bytes"], 65536);
    assert_eq!(
        document["paths"]["/uploads"]["post"]["requestBody"]["content"]["multipart/form-data"]["encoding"]
            ["preview"]["contentType"],
        "text/plain, text/markdown"
    );
    assert!(document["paths"]["/uploads"]["post"]["responses"]["413"].is_object());

    let response = app.oneshot_get("/reference").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );

    assert_eq!(
        app.oneshot_get("/openapi.json").await.status(),
        StatusCode::NOT_FOUND
    );
}
