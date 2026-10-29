pub mod auth;
pub mod domain;
pub mod error;
pub mod routes;
pub mod state;

pub use error::ApiError;
pub use state::AppState;

use roundhouse::openapi::Info;
use tower_http::trace::TraceLayer;

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state)
        .layer(TraceLayer::new_for_http())
        .openapi(|| Info::new("Blog API", env!("CARGO_PKG_VERSION")))
}

/// The same declarations as [`app`], resolved without constructing `AppState`.
pub fn document() -> roundhouse::Document {
    roundhouse::Document::new()
        .scope(module_path!())
        .openapi(|| Info::new("Blog API", env!("CARGO_PKG_VERSION")))
}
