use roundhouse::prelude::*;

use crate::domain::{AuthorPostId, Post};
use crate::error::ApiError;
use crate::state::AppState;

routes! {
    get(
        "/authors/{author_id}/posts/{post_id}",
        show_author_post,
        summary = "Read one author's post",
    ),
}

#[handler]
pub async fn show_author_post(
    State(state): State<AppState>,
    Path(AuthorPostId { author_id, post_id }): Path<AuthorPostId>,
) -> Result<Json<Post>, ApiError> {
    state
        .get_for_author(author_id, post_id)
        .map(Json)
        .ok_or(ApiError::AuthorPostNotFound { author_id, post_id })
}
