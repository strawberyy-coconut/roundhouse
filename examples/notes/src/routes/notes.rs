use roundhouse::prelude::*;

use crate::domain::{NewNote, Note, NoteId};
use crate::error::InternalError;
use crate::state::AppState;

/// The endpoint's outcomes: each variant is a status this operation can actually answer.
#[derive(ApiResponse)]
pub enum ShowNote {
    #[status(200, description = "The stored note.")]
    Found(Json<Note>),

    /// @description: The requested note does not exist.
    #[status(404, description = "The requested note does not exist.")]
    NotFound(Problem),
}

/// The success response for `POST /notes`: exactly one variant, one status, one body.
#[derive(ApiResponse)]
pub enum CreateNote {
    #[status(201, description = "Created")]
    Created {
        #[body]
        body: Json<Note>,
    },
}

/// @summary: List notes
#[handler]
pub async fn list(State(app): State<AppState>) -> Json<Vec<Note>> {
    Json(app.list())
}

/// @summary: Create a note
#[handler]
pub async fn create(
    State(app): State<AppState>,
    Json(body): Json<NewNote>,
) -> Result<CreateNote, InternalError> {
    let note = app.create(body.title, body.body);
    Ok(CreateNote::Created { body: Json(note) })
}

/// @summary: Read a note
#[handler]
pub async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> Result<ShowNote, InternalError> {
    // The in-memory store cannot fail today; `InternalError` is the shared infrastructure side
    // that a real store would return with `?`.
    Ok(app
        .get(id)
        .map(Json)
        .map(ShowNote::Found)
        .unwrap_or_else(|| {
            ShowNote::NotFound(
                Problem::new(axum::http::StatusCode::NOT_FOUND, "Note not found")
                    .type_uri("https://roundhouse.rs/problems/NoteNotFound")
                    .detail(format!("note {id} was not found")),
            )
        }))
}
