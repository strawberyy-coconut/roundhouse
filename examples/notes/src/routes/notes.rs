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
#[handler]
pub async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> impl IntoResponse {
    match app.get(id) {
        Some(note) => Json(note).into_response(),
        None => Problem::not_found("Note not found")
            .type_uri("https://roundhouse.rs/problems/NoteNotFound")
            .detail(format!("note {id} was not found"))
            .into_response(),
    }
}
