use roundhouse::prelude::*;

use crate::domain::{NewNote, Note, NoteId};
use crate::state::AppState;

/// @summary: List notes
#[handler]
pub async fn list(State(app): State<AppState>) -> Json<Vec<Note>> {
    Json(app.list())
}

/// @summary: Create a note
#[handler]
pub async fn create(State(app): State<AppState>, Json(body): Json<NewNote>) -> Created<Note> {
    let note = app.create(body.title, body.body);
    Created(note)
}

/// @summary: Read a note
/// @response 200: The stored note.
/// @response 404: The requested note does not exist.
#[allow(clippy::result_large_err)]
#[handler(response(404, Problem))]
pub async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> Result<Json<Note>, Problem> {
    match app.get(id) {
        Some(note) => Ok(Json(note)),
        None => Err(Problem::not_found("Note not found")
            .type_uri("https://roundhouse.rs/problems/NoteNotFound")
            .detail(format!("note {id} was not found"))),
    }
}
