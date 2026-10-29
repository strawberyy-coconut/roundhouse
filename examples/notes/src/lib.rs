pub mod domain;
pub mod error;
pub mod routes;
pub mod state;

pub use error::ApiError;
pub use state::AppState;

use roundhouse::openapi::Info;
use tower_http::trace::TraceLayer;

/// The application, ready to serve or to mount inside a larger axum router.
pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state)
        .layer(TraceLayer::new_for_http())
        .openapi(|| Info::new("Notes API", env!("CARGO_PKG_VERSION")))
}
