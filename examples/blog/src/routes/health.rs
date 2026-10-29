use roundhouse::prelude::*;

routes! {
    get("/healthz", healthz, summary = "Liveness probe"),
}

#[handler]
pub async fn healthz() -> &'static str {
    "ok"
}
