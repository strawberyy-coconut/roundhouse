use roundhouse::prelude::*;

use crate::domain::Note;
use crate::state::AppState;

routes! {
    get("/", list, summary = "List notes"),
}

#[handler]
pub async fn list(State(app): State<AppState>) -> Json<Vec<Note>> {
    Json(app.list())
}
