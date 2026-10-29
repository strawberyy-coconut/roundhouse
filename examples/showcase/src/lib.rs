pub mod routes;

use std::sync::Arc;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use roundhouse::openapi::Info;

#[derive(Clone, Default)]
pub struct AppState;

async fn inject_extension(mut request: Request, next: Next) -> Response {
    request.extensions_mut().insert(Arc::new(42usize));
    next.run(request).await
}

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state)
        .layer(middleware::from_fn(inject_extension))
        .validation_status(StatusCode::UNPROCESSABLE_ENTITY)
        .document_rejections(false)
        .openapi(|| Info::new("Showcase API", env!("CARGO_PKG_VERSION")))
        .docs()
        .validate_responses()
}
