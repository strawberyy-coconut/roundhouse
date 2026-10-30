use roundhouse::prelude::*;
use serde::Serialize;

routes! {
    prefix = "/internal",
    hidden,
    tags = ["internal"],
    get("/health", health, summary = "Internal health"),
}

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct InternalHealth {
    pub ok: bool,
    pub queue_depth: usize,
}

#[handler]
pub async fn health() -> Json<InternalHealth> {
    Json(InternalHealth {
        ok: true,
        queue_depth: 0,
    })
}
