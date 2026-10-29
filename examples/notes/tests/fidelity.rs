use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use notes_example::AppState;
use notes_example::domain::Note;

fn state() -> AppState {
    AppState::default()
}

fn discovered() -> roundhouse::Built {
    notes_example::app(state()).build()
}

fn hand_written(state: AppState) -> Router {
    use notes_example::routes::{health, notes};
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/notes", get(notes::list::list).post(notes::create::create))
        .route("/notes/{id}", get(notes::show::show))
        .fallback(|| async { roundhouse::Problem::not_found("no route matched this request") })
        .method_not_allowed_fallback(|| async {
            roundhouse::Problem::new(StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed")
                .detail("this path does not accept the request method")
        })
        .with_state(state)
}

async fn request(
    router: Router,
    method: http::Method,
    uri: &str,
    body: Option<&str>,
) -> (StatusCode, Option<String>, Vec<u8>) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(body) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        let request = builder.body(Body::from(body.to_string())).unwrap();
        let response = tower::ServiceExt::oneshot(router, request).await.unwrap();
        return collect(response).await;
    }
    let request = builder.body(Body::empty()).unwrap();
    let response = tower::ServiceExt::oneshot(router, request).await.unwrap();
    collect(response).await
}

async fn collect(response: axum::response::Response) -> (StatusCode, Option<String>, Vec<u8>) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, content_type, body)
}

#[tokio::test]
async fn discovered_and_hand_written_routers_agree() {
    let discovered = discovered();
    let requests = [
        (http::Method::GET, "/healthz", None),
        (http::Method::GET, "/notes", None),
        (
            http::Method::POST,
            "/notes",
            Some(r#"{"title":"Groceries"}"#),
        ),
        (http::Method::GET, "/notes/1", None),
        (http::Method::GET, "/notes/99", None),
        (http::Method::PUT, "/notes", None),
        (http::Method::GET, "/missing", None),
        (http::Method::POST, "/notes", Some(r#"{"title":""}"#)),
    ];

    let hand = hand_written(state());
    for (method, uri, body) in requests {
        let expected = request(hand.clone(), method.clone(), uri, body).await;
        let actual = request(discovered.router(), method.clone(), uri, body).await;
        assert_eq!(actual.0, expected.0, "{method} {uri} status");
        assert_eq!(actual.1, expected.1, "{method} {uri} content-type");
        assert_eq!(actual.2, expected.2, "{method} {uri} body");
    }
}

#[tokio::test]
async fn app_returns_a_real_axum_router() {
    let router: Router<AppState> = notes_example::app(state()).router();
    let response = router.with_state(state()).oneshot_request("/healthz").await;
    assert_eq!(response.status(), StatusCode::OK);
}

trait OneshotExt {
    async fn oneshot_request(self, uri: &str) -> axum::response::Response;
}

impl OneshotExt for Router {
    async fn oneshot_request(self, uri: &str) -> axum::response::Response {
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        tower::ServiceExt::oneshot(self, request).await.unwrap()
    }
}

#[tokio::test]
async fn create_is_created_and_documented() {
    let app = discovered();
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/notes",
            r#"{"title":"Groceries","body":"milk"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let document = app.document();
    let json = serde_json::to_value(document).unwrap();
    assert_eq!(json["openapi"], "3.1.0");
    assert!(json["paths"]["/notes"]["post"]["responses"]["201"].is_object());
    assert!(
        json["paths"]["/notes"]["post"]["responses"]
            .get("200")
            .is_none()
    );
    assert_eq!(
        json["paths"]["/notes"]["post"]["responses"]["201"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/Note"
    );
    assert!(json["components"]["schemas"]["Note"].is_object());
    assert_eq!(
        json["components"]["schemas"]["NewNote"]["properties"]["title"]["minLength"],
        1
    );
    assert_eq!(
        json["paths"]["/notes"]["post"]["requestBody"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/NewNote"
    );
}

#[tokio::test]
async fn openapi_is_served() {
    let app = discovered();
    let response = app.oneshot_get("/openapi.json").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn creating_a_note_and_reading_it_back() {
    let app = discovered();
    let (status, _, _) = request(
        app.router(),
        http::Method::POST,
        "/notes",
        Some(r#"{"title":"Groceries"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let response = app.oneshot_get("/notes/1").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let note: Note = serde_json::from_slice(&body).unwrap();
    assert_eq!(note.title, "Groceries");
}
