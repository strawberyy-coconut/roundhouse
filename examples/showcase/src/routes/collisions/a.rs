use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

routes! {
    get("/a/thing", thing, operation_id = "thing_a"),
}

#[derive(Serialize, Deserialize, ToSchema, ApiSchema)]
#[schema(as = showcase_example::routes::collisions::a::Thing)]
pub struct Thing {
    pub alpha: String,
}

/// @summary: A Thing from module a
#[handler]
pub async fn thing() -> Json<Thing> {
    Json(Thing {
        alpha: "a".to_string(),
    })
}
