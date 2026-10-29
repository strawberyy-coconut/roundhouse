pub mod domain;
pub mod error;
pub mod routes;

pub use error::ApiError;

use roundhouse::openapi::Info;

#[derive(Clone, Default)]
pub struct AppState;

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state).openapi(|| Info::new("Validation API", env!("CARGO_PKG_VERSION")))
}

/// The same declarations as [`app`], resolved without constructing `AppState`.
pub fn document() -> roundhouse::Document {
    roundhouse::Document::new()
        .scope(module_path!())
        .openapi(|| Info::new("Validation API", env!("CARGO_PKG_VERSION")))
}
