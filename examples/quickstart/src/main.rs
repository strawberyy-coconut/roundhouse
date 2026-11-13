//! The smallest roundhouse application.
//!
//! One file: the state alias, the schemas, the route table, the handlers and the server. The
//! multi-file layout is in `examples/notes`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

roundhouse::app_state!(AppState);

#[derive(Clone, Default)]
pub struct AppState {
    inner: Arc<RwLock<Store>>,
}

#[derive(Default)]
struct Store {
    notes: HashMap<i64, Note>,
    next_id: i64,
}

impl AppState {
    fn list(&self) -> Vec<Note> {
        let mut notes: Vec<_> = self.inner.read().unwrap().notes.values().cloned().collect();
        notes.sort_by_key(|note| note.id);
        notes
    }

    fn get(&self, id: i64) -> Option<Note> {
        self.inner.read().unwrap().notes.get(&id).cloned()
    }

    fn create(&self, title: String) -> Note {
        let mut store = self.inner.write().unwrap();
        store.next_id += 1;
        let note = Note {
            id: store.next_id,
            title,
        };
        store.notes.insert(note.id, note.clone());
        note
    }

    fn delete(&self, id: i64) -> bool {
        self.inner.write().unwrap().notes.remove(&id).is_some()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, ApiSchema)]
pub struct Note {
    pub id: i64,
    #[schema(min_length = 1, max_length = 200)]
    pub title: String,
}

#[derive(Deserialize, ToSchema, ApiSchema)]
pub struct NewNote {
    #[schema(min_length = 1, max_length = 200)]
    pub title: String,
}

#[derive(Deserialize, PathParams)]
pub struct NoteId {
    pub id: i64,
}

#[derive(Default, Deserialize, QueryParams)]
pub struct ListQuery {
    #[schema(minimum = 1, maximum = 100)]
    pub limit: Option<usize>,
}

mod routes {
    use super::*;

    routes! {
        prefix = "/v1",
        tags = ["notes"],

        "/health" => {
            get(health),
        },

        "/notes" => {
            get(list),
            post(create),
        },

        "/notes/{id}" => {
            get(show),
            delete(destroy),
        },
    }

    /// @summary: Liveness probe
    #[handler]
    pub async fn health() -> &'static str {
        "ok"
    }

    /// @summary: List notes
    #[handler]
    pub async fn list(
        State(app): State<AppState>,
        Query(query): Query<ListQuery>,
    ) -> Json<Vec<Note>> {
        let mut notes = app.list();
        if let Some(limit) = query.limit {
            notes.truncate(limit);
        }
        Json(notes)
    }

    /// @summary: Create a note
    #[allow(clippy::result_large_err)]
    #[handler(response(201, Note), response_description(201, "The stored note."))]
    pub async fn create(
        State(app): State<AppState>,
        Json(body): Json<NewNote>,
    ) -> Result<Created<Note>, Problem> {
        Ok(Created::new(app.create(body.title)))
    }

    /// @summary: Read a note
    #[handler(path(id, "The note identifier."))]
    pub async fn show(State(app): State<AppState>, Path(NoteId { id }): Path<NoteId>) -> ShowNote {
        match app.get(id) {
            Some(note) => ShowNote::Found(Json(note)),
            None => ShowNote::NotFound(
                Problem::new(roundhouse::http::StatusCode::NOT_FOUND, "Note not found")
                    .type_uri("https://roundhouse.rs/problems/NoteNotFound")
                    .detail(format!("note {id} was not found")),
            ),
        }
    }

    /// @summary: Delete a note
    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn destroy(
        State(app): State<AppState>,
        Path(NoteId { id }): Path<NoteId>,
    ) -> Result<NoContent, Problem> {
        if app.delete(id) {
            Ok(NoContent)
        } else {
            Err(
                Problem::new(roundhouse::http::StatusCode::NOT_FOUND, "Note not found")
                    .type_uri("https://roundhouse.rs/problems/NoteNotFound")
                    .detail(format!("note {id} was not found")),
            )
        }
    }

    /// The endpoint's outcomes: every status this operation can answer.
    #[derive(ApiResponse)]
    pub enum ShowNote {
        #[status(200, description = "The stored note.")]
        Found(Json<Note>),

        #[status(404, description = "The requested note does not exist.")]
        NotFound(Problem),
    }
}

fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state, routes)
        .openapi(|| roundhouse::openapi::Info::new("Quickstart", "0.1.0"))
        .docs()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    app(AppState::default()).serve("0.0.0.0:3000").await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_table_builds() {
        app(AppState::default())
            .try_build()
            .expect("the quickstart route declarations are consistent");
    }

    #[tokio::test]
    async fn create_then_read_a_note() {
        let built = app(AppState::default()).build();

        let response = built
            .oneshot_json(
                roundhouse::http::Method::POST,
                "/v1/notes",
                r#"{"title":"Groceries"}"#,
            )
            .await;
        assert_eq!(response.status(), roundhouse::http::StatusCode::CREATED);

        let response = built.oneshot_get("/v1/notes/1").await;
        assert_eq!(response.status(), roundhouse::http::StatusCode::OK);
        let bytes = roundhouse::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let note: Note = roundhouse::serde_json::from_slice(&bytes).unwrap();
        assert_eq!(note.title, "Groceries");
    }

    #[tokio::test]
    async fn the_document_describes_the_same_routes() {
        let built = app(AppState::default()).build();
        let document = roundhouse::serde_json::to_value(built.document()).unwrap();
        assert!(document["paths"]["/v1/notes"]["get"].is_object());
        assert!(document["paths"]["/v1/notes/{id}"]["delete"].is_object());
        assert_eq!(
            document["paths"]["/v1/notes/{id}"]["get"]["parameters"][0]["description"],
            "The note identifier."
        );
    }
}
