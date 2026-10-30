use axum::body::Body;
use axum::http::{Request, StatusCode};
use roundhouse::App;
use roundhouse::prelude::*;
use roundhouse::utoipa::openapi::Info;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct TestState {
    notes: std::sync::Arc<std::sync::RwLock<Vec<Note>>>,
}

impl TestState {
    fn list(&self) -> Vec<Note> {
        self.notes.read().unwrap().clone()
    }
    fn get(&self, id: i64) -> Option<Note> {
        self.notes
            .read()
            .unwrap()
            .iter()
            .find(|note| note.id == id)
            .cloned()
    }
    fn create(&self, title: String) -> Note {
        let mut notes = self.notes.write().unwrap();
        let id = notes.len() as i64 + 1;
        let note = Note { id, title };
        notes.push(note.clone());
        note
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, ApiSchema)]
pub struct Note {
    pub id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, ApiSchema)]
pub struct NewNote {
    #[schema(min_length = 1, max_length = 120)]
    pub title: String,
}

#[derive(Debug, Deserialize, PathParams)]
pub struct NoteId {
    pub id: i64,
}

#[derive(Debug, thiserror::Error, ApiError)]
pub enum AppError {
    #[error("note not found")]
    #[status(404, problem = "NoteNotFound")]
    NoteNotFound,
}

mod notes {
    use super::*;

    mount!("/notes", tags = ["notes"]);

    routes! {
        get("/", list, summary = "List notes"),
        post("/", create, summary = "Create a note"),
        get("/{id}", show, summary = "Read a note"),
    }

    #[handler]
    pub async fn list(State(state): State<TestState>) -> Json<Vec<Note>> {
        Json(state.list())
    }

    #[handler]
    pub async fn create(
        State(state): State<TestState>,
        Strict(axum::Json(body)): Strict<axum::Json<NewNote>>,
    ) -> Result<Json<Note>, AppError> {
        Ok(Json(state.create(body.title)))
    }

    #[handler]
    pub async fn show(
        State(state): State<TestState>,
        Path(NoteId { id }): Path<NoteId>,
    ) -> Result<Json<Note>, AppError> {
        state.get(id).map(Json).ok_or(AppError::NoteNotFound)
    }
}

mod health {
    use roundhouse::prelude::*;

    mount!(tags = ["ops"]);
    routes! { get("/healthz", healthz, tags = ["ops"]) }

    #[handler]
    pub async fn healthz() -> &'static str {
        "ok"
    }
}

fn app() -> roundhouse::Built {
    App::new(TestState::default())
        .scope(module_path!())
        .openapi(|| Info::new("Test API", "0.1.0"))
        .build()
}

#[tokio::test]
async fn serves_declared_routes() {
    let app = app();
    let response = app.oneshot_get("/healthz").await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = app.oneshot_get("/notes").await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot_json(http::Method::POST, "/notes", r#"{"title":"hello"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = app.oneshot_get("/notes/1").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn validation_is_enforced_and_problem_shaped() {
    let app = app();
    let response = app
        .oneshot_json(http::Method::POST, "/notes", r#"{"title":""}"#)
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(content_type, "application/problem+json");
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["errors"][0]["pointer"], "/title");
    assert_eq!(value["errors"][0]["code"], "min_length");
}

#[tokio::test]
async fn error_enum_is_into_response() {
    let app = app();
    let response = app.oneshot_get("/notes/99").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn document_matches_the_router() {
    let app = app();
    let document = app.document();
    let json = serde_json::to_value(document).unwrap();
    assert_eq!(json["openapi"], "3.1.0");
    assert!(json["paths"]["/notes"]["get"].is_object());
    assert!(json["paths"]["/notes"]["post"].is_object());
    assert!(json["paths"]["/notes/{id}"]["get"].is_object());
    assert_eq!(
        json["paths"]["/notes/{id}"]["get"]["parameters"][0]["name"],
        "id"
    );
    assert_eq!(
        json["paths"]["/notes/{id}"]["get"]["parameters"][0]["in"],
        "path"
    );
    assert!(json["components"]["schemas"]["Note"].is_object());
    assert!(json["components"]["schemas"]["NewNote"].is_object());
    assert_eq!(
        json["paths"]["/notes"]["post"]["responses"]["404"]["content"]["application/problem+json"]
            ["schema"]["type"],
        "object"
    );
}

#[tokio::test]
async fn unmatched_is_json_404() {
    let app = app();
    let response = app.oneshot_get("/nope").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let request = Request::builder().uri("/nope").body(Body::empty()).unwrap();
    let response = app.oneshot(request).await;
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/problem+json"
    );
}

#[test]
fn scope_matching_is_segment_aware() {
    assert!(roundhouse::prefix::in_scope("app", "app::users"));
    assert!(!roundhouse::prefix::in_scope("app", "application::users"));
}

#[test]
fn openapi_golden() {
    let json = app().openapi_json();
    if std::env::var_os("ROUNDHOUSE_BLESS").is_some() {
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/openapi.golden.json"),
            &json,
        )
        .unwrap();
        return;
    }
    let expected = include_str!("openapi.golden.json");
    assert_eq!(json, expected);
}
