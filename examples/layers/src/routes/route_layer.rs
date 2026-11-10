use axum::middleware;
use roundhouse::prelude::*;

routes! {
    prefix = "/route-layer",
    layer = middleware::from_fn(crate::headers::route_layer),
    layer_mode = route,
    get("/ok", ok,
        operation_id = "route_layer_ok"),
}

/// @summary: A route under a route_layer module
#[handler]
pub async fn ok() -> &'static str {
    "route-layer"
}
