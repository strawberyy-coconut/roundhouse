use roundhouse::prelude::*;

routes! {
    "/metrics" => { get(metrics, hidden) },
}

/// Served, but deliberately absent from the OpenAPI document.
/// @summary: Internal metrics
#[handler]
pub async fn metrics() -> &'static str {
    "posts_total 0\n"
}
