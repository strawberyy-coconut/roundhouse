use roundhouse::prelude::*;

routes! {
    tags = ["ops"],
    "/healthz" => { get(healthz) },
}

/// @summary: Liveness probe
#[handler]
pub async fn healthz() -> &'static str {
    "ok"
}
