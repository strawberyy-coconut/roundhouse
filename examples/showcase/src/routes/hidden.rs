use roundhouse::prelude::*;
use serde::Serialize;

routes! {
    prefix = "/internal",
    hidden,
    tags = ["internal"],
    get("/health", health),
}

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct InternalHealth {
    pub ok: bool,
    pub queue_depth: usize,
}

/// @summary: Internal health
#[handler]
pub async fn health() -> Json<InternalHealth> {
    Json(InternalHealth {
        ok: true,
        queue_depth: 0,
    })
}
