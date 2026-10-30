//! `#[derive(ApiResponse)]` / `#[derive(ApiError)]` expansion.

mod attrs;
mod expand;

pub use expand::expand;
