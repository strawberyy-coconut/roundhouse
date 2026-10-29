use axum::middleware;
use roundhouse::prelude::*;

mount!("/entry");

routes! {
    get(
        "/ok",
        ok,
        summary = "A route with its own entry layer",
        layer = middleware::from_fn(crate::headers::entry),
    ),
}

#[handler]
pub async fn ok() -> &'static str {
    "entry"
}
