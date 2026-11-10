use roundhouse::prelude::*;
use serde::Serialize;

use crate::schemes::BearerAuth;

// `security` names the scheme *type*: the guard is installed on every route in this module,
// and the same declaration publishes the requirement in the document.
routes! {
    prefix = "/guarded",
    security = [BearerAuth],
    "/whoami" => { get(whoami) },
}

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Guarded {
    pub message: String,
}

/// @summary: Bearer required by the module
#[handler]
pub async fn whoami() -> Json<Guarded> {
    Json(Guarded {
        message: "guarded".to_string(),
    })
}
