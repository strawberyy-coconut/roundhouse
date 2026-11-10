use roundhouse::prelude::*;

/// The crate-wide infrastructure error.
///
/// Endpoint outcomes live on the `Ok` side (see `ShowNote` and `CreateNote`); this type is only
/// for failures the endpoint cannot describe as a domain result. `Result<T, InternalError>` then
/// documents the outcome statuses plus one shared `500`, instead of every domain error on every
/// route.
#[derive(Debug, thiserror::Error, ApiError)]
pub enum InternalError {
    #[error("internal error: {0}")]
    #[status(500)]
    Internal(String),

    #[error("note storage failed: {0}")]
    #[status(500)]
    Storage(#[from] std::io::Error),
}
