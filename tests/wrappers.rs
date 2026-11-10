//! Reusable `Created<T>` and `NoContent` responses.

use roundhouse::prelude::*;
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Thing {
    pub id: u64,
}

mod routes {
    use super::*;

    #[handler]
    pub async fn create() -> Created<Thing> {
        Created::new(Thing { id: 1 })
    }

    #[handler]
    pub async fn destroy() -> NoContent {
        NoContent
    }

    routes! {
        prefix = "/things",
        "/" => {
            post(create),
        },
        "/{id}" => {
            delete(destroy),
        },
    }
}

#[test]
fn wrappers_document_their_statuses() {
    let built = roundhouse::App::new(State)
        .include(&routes::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Wrappers", "0.1.0"))
        .build();
    let document = serde_json::to_value(built.document()).unwrap();
    assert_eq!(
        document["paths"]["/things"]["post"]["responses"]["201"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/Thing"
    );
    assert_eq!(
        document["paths"]["/things/{id}"]["delete"]["responses"]["204"]["description"],
        "No Content"
    );
}

#[tokio::test]
async fn wrappers_answer_the_status() {
    let built = roundhouse::App::new(State).include(&routes::MODULE).build();
    let created = built
        .oneshot_json(roundhouse::http::Method::POST, "/things", r#"{}"#)
        .await;
    assert_eq!(created.status(), axum::http::StatusCode::CREATED);

    let request = roundhouse::http::Request::builder()
        .method(roundhouse::http::Method::DELETE)
        .uri("/things/1")
        .body(axum::body::Body::empty())
        .unwrap();
    let deleted = built.oneshot(request).await;
    assert_eq!(deleted.status(), axum::http::StatusCode::NO_CONTENT);
}
