use roundhouse::prelude::*;

/// @summary: Liveness probe
#[handler]
pub async fn healthz() -> &'static str {
    "ok"
}
