//! Response variants that carry data must say where that data goes.

use axum::http::StatusCode;
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

roundhouse::app_state!(State);

#[derive(serde::Serialize, ToSchema, ApiSchema)]
pub struct Payload {
    pub value: i64,
}

#[derive(ApiResponse)]
pub enum Outcome {
    #[status(200)]
    Found(Json<Payload>),

    #[status(404, description = "The requested value does not exist.")]
    Missing(Problem),

    #[status(409, description = "The value already exists.")]
    Conflict {
        /// This field is deliberately not part of the response.
        #[response(ignore)]
        id: i64,
    },
}

mod routes {
    use super::*;

    routes! {
        "/values/{id}" => { get(show) },
    }

    #[handler]
    pub async fn show(Path(id): Path<i64>) -> Outcome {
        if id == 0 {
            Outcome::Missing(
                Problem::new(StatusCode::NOT_FOUND, "Value not found")
                    .type_uri("https://roundhouse.rs/problems/ValueNotFound")
                    .detail(format!("value {id} was not found")),
            )
        } else {
            Outcome::Found(Json(Payload { value: id }))
        }
    }
}

fn app() -> roundhouse::App<State> {
    roundhouse::app!(State, routes).openapi(|| roundhouse::openapi::Info::new("Outcome", "0.1.0"))
}

#[tokio::test]
async fn a_problem_body_sets_the_status_and_detail() {
    let built = app().build();
    let response = built.oneshot_get("/values/0").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.headers()[axum::http::header::CONTENT_TYPE],
        "application/problem+json"
    );
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["type"],
        "https://roundhouse.rs/problems/ValueNotFound"
    );
    assert_eq!(value["detail"], "value 0 was not found");
}

#[tokio::test]
async fn the_success_variant_still_serves_json() {
    let built = app().build();
    let response = built.oneshot_get("/values/7").await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["value"], 7);
}
