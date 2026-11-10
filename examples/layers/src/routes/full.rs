use axum::middleware;
use roundhouse::prelude::*;

routes! {
    prefix = "/full",
    layer = middleware::from_fn(crate::headers::module),
    "/ok" => { get(ok,
        operation_id = "full_ok") },
}

/// @summary: A route under a full module layer
#[handler]
pub async fn ok() -> &'static str {
    "full"
}
