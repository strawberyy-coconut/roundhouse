pub mod headers;
pub mod routes;

use axum::Router;
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::get;
use roundhouse::openapi::Info;

#[derive(Clone, Default)]
pub struct AppState;

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state)
        .layer(middleware::from_fn(headers::global))
        .fallback(|| async { (StatusCode::IM_A_TEAPOT, "custom fallback") })
        .merge(merged_router())
}

/// A hand-written router merged into the discovered app.
fn merged_router() -> Router {
    Router::new().route("/merged", get(|| async { "merged" }))
}

/// The other escape hatch: take the ready router and embed it inside a larger axum app.
pub fn embedded(state: AppState) -> Router {
    let discovered = app(state.clone()).router();
    Router::new()
        .nest("/embedded", discovered)
        .route("/outer", get(|| async { "outer" }))
        .with_state(state)
}

/// The same declarations as [`app`], resolved without constructing `AppState`.
pub fn document() -> roundhouse::Document {
    roundhouse::Document::new()
        .scope(module_path!())
        .openapi(|| Info::new("Layers API", env!("CARGO_PKG_VERSION")))
}
