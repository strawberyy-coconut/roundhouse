use axum::middleware;
use roundhouse::prelude::*;

routes! {
    prefix = "/entry",
    get(
        "/ok",
        ok,
        summary = "A route with its own entry layer",
        operation_id = "entry_ok",
        layer = middleware::from_fn(crate::headers::entry),
    ),
}

#[handler]
pub async fn ok() -> &'static str {
    "entry"
}
