use roundhouse::prelude::*;

use crate::domain::{Post, PostId, PostStatus, UpdatePost};
use crate::error::ApiError;
use crate::state::AppState;

routes! {
    "/{id}" => {
        get(show, summary = "Read a post"),
        put(update, summary = "Replace a post"),
        delete(destroy, summary = "Delete a post", responses(204)),
    },
    "/{id}/status" => {
        patch(set_status, summary = "Change a post's status", responses(204)),
    },
}

#[handler]
pub async fn show(
    State(state): State<AppState>,
    Path(PostId { id }): Path<PostId>,
) -> Result<Json<Post>, ApiError> {
    state.get(id).map(Json).ok_or(ApiError::PostNotFound { id })
}

#[handler]
pub async fn update(
    State(state): State<AppState>,
    Path(PostId { id }): Path<PostId>,
    Json(body): Json<UpdatePost>,
) -> Result<Json<Post>, ApiError> {
    state
        .update(id, body)
        .map(Json)
        .ok_or(ApiError::PostNotFound { id })
}

#[handler]
pub async fn destroy(
    State(state): State<AppState>,
    Path(PostId { id }): Path<PostId>,
) -> Result<StatusCode, ApiError> {
    if state.delete(id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::PostNotFound { id })
    }
}

#[handler]
pub async fn set_status(
    State(state): State<AppState>,
    Path(PostId { id }): Path<PostId>,
    Json(status): Json<PostStatus>,
) -> Result<StatusCode, ApiError> {
    state
        .set_status(id, status)
        .map(|_| StatusCode::NO_CONTENT)
        .ok_or(ApiError::PostNotFound { id })
}
