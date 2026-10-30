use roundhouse::prelude::*;

routes! {
    tags = ["ops"],
    get("/healthz", healthz, summary = "Liveness probe"),
}

#[handler]
pub async fn healthz() -> &'static str {
    "ok"
}
