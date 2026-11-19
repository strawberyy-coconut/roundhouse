//! Generated status wrappers and the universal `Status`/`Empty` primitives.

use axum::http::StatusCode;
use roundhouse::interop::ToSchema;
use roundhouse::prelude::*;
use roundhouse::status::{Conflict, NotFound, Success};
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Thing {
    pub id: u64,
}

mod routes {
    use super::*;

    routes! {
        prefix = "/status",
        "/success" => { get(success, operation_id = "status_success") },
        "/generated" => { get(generated, operation_id = "status_generated") },
        "/conflict" => { get(conflict, operation_id = "status_conflict") },
        "/custom" => { get(custom, operation_id = "status_custom") },
        "/empty" => { get(empty, operation_id = "status_empty") },
    }

    #[handler]
    pub async fn success() -> Success<Thing> {
        Success::new(Thing { id: 1 })
    }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn generated() -> Result<Json<Thing>, NotFound<Problem>> {
        Err(NotFound(Problem::not_found("missing")))
    }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn conflict() -> Result<Json<Thing>, Conflict<Problem>> {
        Err(Conflict(Problem::conflict("duplicate")))
    }

    #[handler]
    pub async fn custom() -> Status<207, Thing> {
        Status::new(Thing { id: 1 })
    }

    #[handler]
    pub async fn empty() -> Empty<204> {
        Empty::new()
    }
}

fn app() -> roundhouse::Built {
    roundhouse::App::new(State)
        .include(&routes::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Status codes", "0.1.0"))
        .try_build()
        .expect("the status wrappers document cleanly")
}

#[test]
fn generated_wrappers_document_their_statuses() {
    let document = serde_json::to_value(app().document()).unwrap();
    let paths = &document["paths"];

    assert!(paths["/status/success"]["get"]["responses"]["200"].is_object());
    assert!(paths["/status/generated"]["get"]["responses"]["200"].is_object());
    assert!(paths["/status/generated"]["get"]["responses"]["404"].is_object());
    assert!(paths["/status/conflict"]["get"]["responses"]["409"].is_object());
    assert!(paths["/status/custom"]["get"]["responses"]["207"].is_object());
    assert!(paths["/status/empty"]["get"]["responses"]["204"].is_object());

    assert_eq!(
        paths["/status/generated"]["get"]["responses"]["404"]["content"]["application/json"]["schema"]
            ["properties"]["title"]["type"],
        "string"
    );
}

#[tokio::test]
async fn wrappers_answer_the_status_they_document() {
    let built = app();

    let success = built.oneshot_get("/status/success").await;
    assert_eq!(success.status(), StatusCode::OK);

    let generated = built.oneshot_get("/status/generated").await;
    assert_eq!(generated.status(), StatusCode::NOT_FOUND);

    let conflict = built.oneshot_get("/status/conflict").await;
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let custom = built.oneshot_get("/status/custom").await;
    assert_eq!(custom.status(), StatusCode::MULTI_STATUS);

    let empty = built.oneshot_get("/status/empty").await;
    assert_eq!(empty.status(), StatusCode::NO_CONTENT);
}

roundhouse::app_state!(State);
