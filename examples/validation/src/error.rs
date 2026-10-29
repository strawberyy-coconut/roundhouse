use roundhouse::prelude::*;

#[derive(Debug, thiserror::Error, ApiError)]
pub enum ApiError {
    #[error("username `{username}` is reserved")]
    #[status(409, problem = "UsernameReserved")]
    UsernameReserved { username: String },
}
