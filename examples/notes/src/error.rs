use roundhouse::prelude::*;

/// Every failure the API can produce, with the status and problem `type` the document promises.
#[derive(Debug, thiserror::Error, ApiError)]
pub enum ApiError {
    #[error("note {id} was not found")]
    #[status(404, problem = "NoteNotFound")]
    NoteNotFound { id: i64 },

    #[error("invalid input")]
    #[status(422)]
    Validation(#[from] ValidationErrors),

    #[error("internal error: {0}")]
    #[status(500)]
    Internal(String),
}
