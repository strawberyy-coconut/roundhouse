use axum::middleware;
use roundhouse::prelude::*;

routes! {
    prefix = "/entry",
    get(
        "/ok",
        ok,
        operation_id = "entry_ok",
        layer = middleware::from_fn(crate::headers::entry),
    ),
}

/// @summary: A route with its own entry layer
#[handler]
pub async fn ok() -> &'static str {
    "entry"
}
