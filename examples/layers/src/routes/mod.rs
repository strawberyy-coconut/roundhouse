use axum::middleware;
use roundhouse::prelude::*;

/// @summary: A route under a full module layer
#[handler]
pub async fn full_ok() -> &'static str {
    "full"
}

/// @summary: A route under a route_layer module
#[handler]
pub async fn route_layer_ok() -> &'static str {
    "route-layer"
}

/// @summary: A route with its own entry layer
#[handler]
pub async fn entry_ok() -> &'static str {
    "entry"
}

routes! {
    "/full" [layer = middleware::from_fn(crate::headers::module)] {
        "/ok" => {
            get(full_ok),
        },
    },

    "/route-layer" [
        layer = middleware::from_fn(crate::headers::route_layer),
        layer_mode = route,
    ] {
        "/ok" => {
            get(route_layer_ok),
        },
    },

    "/entry" {
        "/ok" => {
            get(entry_ok, layer = middleware::from_fn(crate::headers::entry)),
        },
    },
}
