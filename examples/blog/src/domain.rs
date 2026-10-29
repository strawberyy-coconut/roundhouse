use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostStatus {
    Draft,
    Published,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: i64,
    pub author_id: i64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub status: PostStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct NewPost {
    pub author_id: i64,
    #[schema(min_length = 3, max_length = 100)]
    pub title: String,
    pub body: String,
    #[schema(min_items = 1, max_items = 5, unique_items)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePost {
    #[schema(min_length = 3, max_length = 100)]
    pub title: String,
    pub body: String,
    #[schema(min_items = 1, max_items = 5, unique_items)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PathParams)]
pub struct PostId {
    #[schema(minimum = 1)]
    pub id: i64,
}

#[derive(Debug, Clone, Copy, Deserialize, PathParams)]
pub struct AuthorPostId {
    #[schema(minimum = 1)]
    pub author_id: i64,
    pub post_id: i64,
}
