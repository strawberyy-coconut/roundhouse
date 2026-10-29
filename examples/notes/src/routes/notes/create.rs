use roundhouse::prelude::*;

use crate::domain::{NewNote, Note};
use crate::error::ApiError;
use crate::state::AppState;

routes! {
    post("/", create, summary = "Create a note"),
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

#[handler]
pub async fn create(
    State(app): State<AppState>,
    Json(body): Json<NewNote>,
) -> Result<CreateNote, ApiError> {
    let note = app.create(body.title, body.body);
    Ok(CreateNote::Created { body: Json(note) })
}
