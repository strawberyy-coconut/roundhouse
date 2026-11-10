use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

routes! {
    "/b/thing" => { get(thing, operation_id = "thing_b") },
}

#[derive(Serialize, Deserialize, ToSchema, ApiSchema)]
#[schema(as = showcase_example::routes::collisions::b::Thing)]
pub struct Thing {
    pub beta: i64,
}

/// @summary: A Thing from module b
#[handler]
pub async fn thing() -> Json<Thing> {
    Json(Thing { beta: 2 })
}
