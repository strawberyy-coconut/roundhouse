use axum::middleware;
use roundhouse::prelude::*;

routes! {
    prefix = "/full",
    layer = middleware::from_fn(crate::headers::module),
    get("/ok", ok, summary = "A route under a full module layer",
        operation_id = "full_ok"),
}

#[handler]
pub async fn ok() -> &'static str {
    "full"
}
