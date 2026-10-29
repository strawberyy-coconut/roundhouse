use roundhouse::prelude::*;
use serde::Serialize;

use crate::schemes::BearerAuth;

// `security` names the scheme *type*: the guard below is installed on every route in this
// module, and the same declaration publishes the requirement in the document.
mount!("/guarded", security = [BearerAuth]);

routes! {
    get("/whoami", whoami, summary = "Bearer required by the mount"),
}

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Guarded {
    pub message: String,
}

#[handler]
pub async fn whoami() -> Json<Guarded> {
    Json(Guarded {
        message: "guarded".to_string(),
    })
}
