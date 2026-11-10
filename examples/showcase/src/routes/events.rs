use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

routes! {
    "/events" => { post(create) },
}

#[derive(Debug, Serialize, Deserialize, ToSchema, ApiSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Created {
        #[schema(min_length = 3)]
        title: String,
    },
    Deleted,
}

/// @summary: Validate a tagged event
#[handler]
pub async fn create(Json(event): Json<Event>) -> Json<Event> {
    Json(event)
}
