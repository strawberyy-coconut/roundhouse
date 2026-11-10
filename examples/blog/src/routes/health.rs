use roundhouse::prelude::*;

routes! {
    "/healthz" => { get(healthz) },
}

/// @summary: Liveness probe
#[handler]
pub async fn healthz() -> &'static str {
    "ok"
}
