use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use roundhouse::Built;
use roundhouse::prelude::*;
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

mod files {
    use super::*;

    mount!("/files", tags = ["files"]);

    routes! {
        post("/upload", upload, summary = "Upload a file"),
        get("/download", download, summary = "Download bytes"),
        get("/raw", raw, summary = "Download a file"),
        get("/events", events, summary = "Server-sent events"),
        post("/strict", strict),
    }

    #[derive(Debug, roundhouse::Multipart)]
    pub struct Upload {
        pub file: FilePart,
        #[file(max_bytes = 5, accept = ["text/plain"])]
        pub thumbnail: Option<FilePart>,
        #[file(max_bytes = 100)]
        pub temp: Option<TempFile>,
        pub tags: Vec<String>,
        pub caption: Option<String>,
    }

    #[derive(Serialize, ApiSchema)]
    pub struct UploadSummary {
        pub file_name: Option<String>,
        pub file_len: usize,
        pub thumbnail: Option<String>,
        pub temp_len: Option<u64>,
        pub tags: Vec<String>,
        pub caption: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(Debug, roundhouse::Multipart)]
    #[multipart(deny_unknown)]
    pub struct StrictUpload {
        pub file: FilePart,
    }

    #[handler]
    pub async fn strict(Multipart(_form): Multipart<StrictUpload>) -> StatusCode {
        StatusCode::NO_CONTENT
    }

    #[handler]
    pub async fn upload(Multipart(form): Multipart<Upload>) -> Json<UploadSummary> {
        Json(UploadSummary {
            file_name: form.file.file_name.clone(),
            file_len: form.file.len(),
            thumbnail: form.thumbnail.as_ref().map(|file| file.len().to_string()),
            temp_len: form.temp.as_ref().map(|file| file.len()),
            tags: form.tags,
            caption: form.caption,
        })
    }

    #[handler]
    pub async fn download() -> Attachment<Vec<u8>> {
        Attachment::new(b"hello".to_vec()).filename("hello.txt")
    }

    #[handler]
    pub async fn raw() -> File {
        File::new(vec![1u8, 2, 3]).content_type("application/octet-stream")
    }

    #[handler]
    pub async fn events() -> Sse<
        impl futures_util::Stream<Item = ::core::result::Result<Event, ::core::convert::Infallible>>,
    > {
        Sse::new(futures_util::stream::once(async {
            Ok(Event::default().data("hello"))
        }))
    }
}

fn app() -> Built {
    roundhouse::App::new(State)
        .scope(module_path!())
        .openapi(|| roundhouse::openapi::Info::new("Files", "0.1.0"))
        .build()
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

async fn send_multipart(app: &Built, body: &str) -> axum::response::Response {
    let request = Request::builder()
        .method(Method::POST)
        .uri("/files/upload")
        .header(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=test-boundary",
        )
        .body(Body::from(body.to_string()))
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

#[tokio::test]
async fn multipart_uploads_build_a_struct_in_one_pass() {
    let app = app();
    let body = multipart(
        "test-boundary",
        &[
            ("file", Some("a.txt"), Some("text/plain"), "hello"),
            ("thumbnail", Some("thumb.txt"), Some("text/plain"), "tiny"),
            ("temp", Some("temp.txt"), Some("text/plain"), "temp bytes"),
            ("tags", None, None, "rust"),
            ("tags", None, None, "axum"),
            ("caption", None, None, "a caption"),
            ("ignored", None, None, "ignored"),
        ],
    );
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value = body_json(response).await;
    assert_eq!(value["file_name"], "a.txt");
    assert_eq!(value["file_len"], 5);
    assert_eq!(value["thumbnail"], "4");
    assert_eq!(value["temp_len"], 10);
    assert_eq!(value["tags"], serde_json::json!(["rust", "axum"]));
    assert_eq!(value["caption"], "a caption");
}

#[tokio::test]
async fn missing_and_invalid_parts_return_problem_statuses() {
    let app = app();

    let body = multipart("test-boundary", &[("tags", None, None, "rust")]);
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = body_json(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/file");
    assert_eq!(value["errors"][0]["code"], "required");

    let body = multipart(
        "test-boundary",
        &[
            ("file", Some("a.txt"), Some("text/plain"), "hello"),
            (
                "thumbnail",
                Some("thumb.json"),
                Some("application/json"),
                "{}",
            ),
        ],
    );
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let body = multipart(
        "test-boundary",
        &[
            ("file", Some("a.txt"), Some("text/plain"), "hello"),
            (
                "thumbnail",
                Some("thumb.txt"),
                Some("text/plain"),
                "too big",
            ),
        ],
    );
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn attachments_and_raw_files_set_download_headers() {
    let app = app();
    let response = app.oneshot_get("/files/download").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"hello.txt\""
    );

    let response = app.oneshot_get("/files/raw").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );

    let response = app.oneshot_get("/files/events").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
}

#[tokio::test]
async fn multipart_and_attachment_are_documented() {
    let app = app();
    let document = serde_json::to_value(app.document()).unwrap();
    let media = &document["paths"]["/files/upload"]["post"]["requestBody"]["content"]["multipart/form-data"];
    let body = &media["schema"];
    assert_eq!(body["properties"]["file"]["format"], "binary");
    assert_eq!(body["properties"]["tags"]["type"], "array");
    assert_eq!(
        body["properties"]["caption"]["type"],
        serde_json::json!(["string", "null"])
    );
    assert_eq!(media["encoding"]["thumbnail"]["contentType"], "text/plain");
    assert!(media["encoding"].get("file").is_none());
    let responses = &document["paths"]["/files/upload"]["post"]["responses"];
    assert!(responses["400"].is_object());
    assert!(responses["413"].is_object());
    assert!(responses["415"].is_object());

    let download = &document["paths"]["/files/download"]["get"]["responses"]["200"];
    assert_eq!(
        download["content"]["application/octet-stream"]["schema"]["format"],
        "binary"
    );
    assert!(download["headers"]["Content-Disposition"].is_object());
    assert_eq!(
        document["paths"]["/files/events"]["get"]["responses"]["200"]["content"]["text/event-stream"]
            ["schema"]["type"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn deny_unknown_rejects_extra_multipart_fields() {
    let app = app();
    let body = multipart(
        "test-boundary",
        &[
            ("file", Some("a.txt"), Some("text/plain"), "hello"),
            ("extra", None, None, "nope"),
        ],
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri("/files/strict")
        .header(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=test-boundary",
        )
        .body(Body::from(body))
        .unwrap();
    let response = app.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = body_json(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/extra");
    assert_eq!(value["errors"][0]["code"], "unknown");
}

#[tokio::test]
async fn duplicate_single_valued_fields_are_rejected() {
    let app = app();
    let body = multipart(
        "test-boundary",
        &[
            ("file", Some("a.txt"), Some("text/plain"), "hello"),
            ("file", Some("b.txt"), Some("text/plain"), "again"),
        ],
    );
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value = body_json(response).await;
    assert_eq!(value["errors"][0]["pointer"], "/file");
    assert_eq!(value["errors"][0]["code"], "duplicate_field");
}

#[tokio::test]
async fn zero_byte_files_are_distinct_from_missing_ones() {
    let app = app();
    let body = multipart(
        "test-boundary",
        &[("file", Some("empty.txt"), Some("text/plain"), "")],
    );
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value = body_json(response).await;
    assert_eq!(value["file_name"], "empty.txt");
    assert_eq!(value["file_len"], 0);
}

#[tokio::test]
async fn accept_matches_the_media_type_with_parameters() {
    let app = app();
    let body = multipart(
        "test-boundary",
        &[
            ("file", Some("a.txt"), Some("text/plain"), "hello"),
            (
                "thumbnail",
                Some("thumb.txt"),
                Some("text/plain; charset=utf-8"),
                "tiny",
            ),
        ],
    );
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_non_multipart_content_type_is_415() {
    let app = app();
    let request = Request::builder()
        .method(Method::POST)
        .uri("/files/upload")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = app.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let value = body_json(response).await;
    assert_eq!(value["status"], 415);
}

#[tokio::test]
async fn max_parts_bounds_the_part_count() {
    let app = roundhouse::App::new(State)
        .scope(module_path!())
        .max_parts(1)
        .build();
    let body = multipart(
        "test-boundary",
        &[
            ("file", Some("a.txt"), Some("text/plain"), "hello"),
            ("tags", None, None, "rust"),
        ],
    );
    let response = send_multipart(&app, &body).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let value = body_json(response).await;
    assert_eq!(value["errors"][0]["code"], "max_parts");
}
