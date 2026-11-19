pub mod domain;
pub mod error;
pub mod routes;
pub mod state;

pub use state::AppState;

roundhouse::app_state!(AppState);

roundhouse::api! {
    state = AppState,
    title = "Notes API",
    version = env!("CARGO_PKG_VERSION"),
    root = routes::MODULE,
}

use tower_http::trace::TraceLayer;

/// The application, ready to serve or to embed inside a larger axum router.
pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state, routes)
        .layer(TraceLayer::new_for_http())
        .contract(roundhouse::include_contract!())
        .openapi_route("/openapi.json")
}
