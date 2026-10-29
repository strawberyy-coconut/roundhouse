use roundhouse::prelude::*;

use crate::domain::{Note, NoteId};
use crate::error::ApiError;
use crate::state::AppState;

routes! {
    get("/{id}", show, summary = "Read a note"),
}

#[handler]
pub async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> Result<Json<Note>, ApiError> {
    app.get(id).map(Json).ok_or(ApiError::NoteNotFound { id })
}
