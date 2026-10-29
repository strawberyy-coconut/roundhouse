use axum::middleware;
use roundhouse::prelude::*;

mount!("/full", layer = middleware::from_fn(crate::headers::module));

routes! {
    get("/ok", ok, summary = "A route under a full module layer"),
}

#[handler]
pub async fn ok() -> &'static str {
    "full"
}
