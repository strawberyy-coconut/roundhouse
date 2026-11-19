use roundhouse::prelude::*;

use crate::domain::{NewNote, Note, NoteId};
use crate::error::{CreateNoteError, InvalidTitle, MissingNote};
use crate::state::AppState;

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
) -> Result<impl IntoResponse, CreateNoteError> {
    // Two different `?` sources: the title check and the store's duplicate check. Both convert
    // into `CreateNoteError`; its `DocumentedError` impl chooses the 422 and 409 responses.
    // The `?` path is the error path; normal returns are responses.
    let title = validate_title(body.title)?;
    let note = app.create(title, body.body)?;
    Ok(Created(note))
}

fn validate_title(title: String) -> Result<String, InvalidTitle> {
    let title = title.trim().to_string();
    if title.is_empty() {
        Err(InvalidTitle)
    } else {
        Ok(title)
    }
}

/// @summary: Read a note
/// @response 200: The stored note.
/// @response 404: The requested note does not exist.
#[handler]
pub async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> Result<impl IntoResponse, MissingNote> {
    // `?` is the error path: it converts into `MissingNote`, whose `DocumentedError` impl
    // chooses the 404 response.
    let note = app.find(id)?;
    Ok(Json(note))
}
