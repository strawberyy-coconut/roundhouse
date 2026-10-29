use roundhouse::prelude::*;
use serde::Deserialize;

use crate::domain::{NewPost, Post};
use crate::error::ApiError;
use crate::state::AppState;

routes! {
    "/" => {
        get(list, summary = "List posts"),
        post(create, summary = "Create a post"),
    },
}

#[derive(Debug, Default, Deserialize, QueryParams)]
pub struct ListQuery {
    pub tag: Option<String>,
    #[schema(minimum = 1, maximum = 100)]
    pub limit: Option<usize>,
}

#[handler]
pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<Post>> {
    Json(state.list(query.tag.as_deref(), query.limit))
}

#[derive(ApiResponse)]
pub enum CreatedPost {
    #[status(201, description = "Created")]
    Created {
        #[body]
        body: Json<Post>,
        #[location]
        location: String,
    },
}

#[handler]
pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<NewPost>,
) -> Result<CreatedPost, ApiError> {
    let post = state.create(body);
    Ok(CreatedPost::Created {
        location: format!("/api/blog/posts/{}", post.id),
        body: Json(post),
    })
}
