use roundhouse::prelude::*;

routes! {
    get("/metrics", metrics, hidden, summary = "Internal metrics"),
}

/// Served, but deliberately absent from the OpenAPI document.
#[handler]
pub async fn metrics() -> &'static str {
    "posts_total 0\n"
}
