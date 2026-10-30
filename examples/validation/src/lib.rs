pub mod domain;
pub mod error;
pub mod routes;

pub use error::ApiError;

use roundhouse::openapi::Info;

#[derive(Clone, Default)]
pub struct AppState;

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state, routes)
        .openapi(|| Info::new("Validation API", env!("CARGO_PKG_VERSION")))
}
