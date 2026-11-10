use roundhouse::prelude::*;

use crate::domain::{NewNote, Note};
use crate::error::InternalError;
use crate::state::AppState;

routes! {
    "/" => { post(create) },
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

/// @summary: Create a note
#[handler]
pub async fn create(
    State(app): State<AppState>,
    Json(body): Json<NewNote>,
) -> Result<CreateNote, InternalError> {
    let note = app.create(body.title, body.body);
    Ok(CreateNote::Created { body: Json(note) })
}
