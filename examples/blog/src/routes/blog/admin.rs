use roundhouse::prelude::*;
use serde::Serialize;

use crate::auth::DemoBearer;

routes! {
    "/admin/audit" => { get(audit) },
    "/admin/optional" => { get(optional) },
}

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct AuditReport {
    pub posts: usize,
    pub secured: bool,
}

/// @summary: Read the audit report
#[handler]
pub async fn audit(_auth: Auth<DemoBearer>) -> Json<AuditReport> {
    Json(AuditReport {
        posts: 0,
        secured: true,
    })
}

/// @summary: An endpoint that works with or without auth
#[handler]
pub async fn optional(auth: Option<Auth<DemoBearer>>) -> &'static str {
    if auth.is_some() {
        "authenticated"
    } else {
        "public"
    }
}
