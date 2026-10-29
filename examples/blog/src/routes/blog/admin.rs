use roundhouse::prelude::*;
use serde::Serialize;

use crate::auth::DemoBearer;

routes! {
    get("/admin/audit", audit, summary = "Read the audit report"),
    get(
        "/admin/optional",
        optional,
        summary = "An endpoint that works with or without auth",
    ),
}

#[derive(Serialize, ApiSchema)]
pub struct AuditReport {
    pub posts: usize,
    pub secured: bool,
}

#[handler]
pub async fn audit(_auth: Auth<DemoBearer>) -> Json<AuditReport> {
    Json(AuditReport {
        posts: 0,
        secured: true,
    })
}

#[handler]
pub async fn optional(auth: Option<Auth<DemoBearer>>) -> &'static str {
    if auth.is_some() {
        "authenticated"
    } else {
        "public"
    }
}
