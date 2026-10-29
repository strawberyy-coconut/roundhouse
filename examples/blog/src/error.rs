use roundhouse::prelude::*;

#[derive(Debug, thiserror::Error, ApiError)]
pub enum ApiError {
    #[error("post {id} was not found")]
    #[status(404, problem = "PostNotFound")]
    PostNotFound { id: i64 },

    #[error("author {author_id} has no post {post_id}")]
    #[status(404, problem = "AuthorPostNotFound")]
    AuthorPostNotFound { author_id: i64, post_id: i64 },
}
