//! `#[from]` on an infrastructure error must both provide the `From` impl and compile without an
//! unused-binding warning.

use axum::http::StatusCode;
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

#[derive(Debug, serde::Serialize, ToSchema, ApiSchema)]
pub struct Thing {
    pub id: u64,
}

#[derive(Debug, thiserror::Error, ApiError)]
pub enum InfraError {
    #[error("io: {0}")]
    #[status(500)]
    Io(#[from] std::io::Error),
}

mod routes {
    use super::*;

    routes! {
        prefix = "/from",
        "/" => { get(fail) },
    }

    #[handler]
    pub async fn fail() -> Result<Json<Thing>, InfraError> {
        Err(InfraError::Io(std::io::Error::other("disk unavailable")))
    }
}

#[tokio::test]
async fn a_shared_infrastructure_error_answers_a_problem() {
    let built = roundhouse::App::new(State)
        .include(&routes::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Errors", "0.1.0"))
        .build();

    let document = serde_json::to_value(built.document()).unwrap();
    assert_eq!(
        document["paths"]["/from"]["get"]["responses"]["500"]["description"],
        "Internal Server Error"
    );

    let response = built.oneshot_get("/from").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(problem["title"], "Internal Server Error");
    assert!(
        problem["detail"]
            .as_str()
            .unwrap()
            .contains("disk unavailable")
    );
}
