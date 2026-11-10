use roundhouse::prelude::*;

use crate::domain::{Note, NoteId};
use crate::error::InternalError;
use crate::state::AppState;

routes! {
    "/{id}" => { get(show) },
}

/// The endpoint's outcomes: each variant is a status this operation can actually answer.
#[derive(ApiResponse)]
pub enum ShowNote {
    #[status(200, description = "The stored note.")]
    Found(Json<Note>),

    /// @description: The requested note does not exist.
    #[status(404, problem = "NoteNotFound")]
    NotFound { id: i64 },
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
        .unwrap_or(ShowNote::NotFound { id }))
}
