use axum::middleware;
use roundhouse::prelude::*;

mount!(
    "/route-layer",
    layer = middleware::from_fn(crate::headers::route_layer),
    layer_mode = route,
);

routes! {
    get("/ok", ok, summary = "A route under a route_layer module",
        operation_id = "route_layer_ok"),
}

#[handler]
pub async fn ok() -> &'static str {
    "route-layer"
}
