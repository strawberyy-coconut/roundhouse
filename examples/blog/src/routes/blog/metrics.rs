use roundhouse::prelude::*;

routes! {
    get("/metrics", metrics, hidden),
}

/// Served, but deliberately absent from the OpenAPI document.
/// @summary: Internal metrics
#[handler]
pub async fn metrics() -> &'static str {
    "posts_total 0\n"
}
