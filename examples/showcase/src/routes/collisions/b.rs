use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

routes! {
    get("/b/thing", thing, summary = "A Thing from module b", operation_id = "thing_b"),
}

#[derive(Serialize, Deserialize, ApiSchema)]
pub struct Thing {
    pub beta: i64,
}

#[handler]
pub async fn thing() -> Json<Thing> {
    Json(Thing { beta: 2 })
}
