//! Generic `ApiResponse` / `ApiError` derives.

use roundhouse::prelude::*;
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

#[derive(Debug, Serialize, ToSchema, ApiSchema)]
pub struct Thing {
    pub id: u64,
}

#[derive(ApiResponse)]
pub enum Maybe<T> {
    #[status(200)]
    Some(#[body] Json<T>),

    #[status(404)]
    Missing,
}

#[derive(ApiResponse)]
pub enum Paged<T, C> {
    #[status(200)]
    Page {
        #[body]
        items: Json<Vec<T>>,
        #[header("x-next-cursor")]
        next: Option<C>,
    },
}

#[derive(Debug, thiserror::Error, ApiError)]
pub enum WrappedError<E: std::error::Error + 'static> {
    #[error("wrapped")]
    #[status(500)]
    Wrapped(#[source] E),
}

mod routes {
    use super::*;

    #[handler]
    pub async fn maybe() -> Maybe<Thing> {
        Maybe::Some(Json(Thing { id: 1 }))
    }

    #[handler]
    pub async fn paged() -> Paged<Thing, String> {
        Paged::Page {
            items: Json(vec![Thing { id: 1 }]),
            next: Some("next".to_string()),
        }
    }

    #[handler]
    pub async fn wrapped() -> Result<Json<Thing>, WrappedError<std::io::Error>> {
        Ok(Json(Thing { id: 1 }))
    }

    routes! {
        prefix = "/generic",
        "/maybe" => {
            get(maybe),
        },
        "/paged" => {
            get(paged),
        },
        "/wrapped" => {
            get(wrapped),
        },
    }
}

#[test]
fn generic_response_enums_document_and_build() {
    let built = roundhouse::App::new(State)
        .include(&routes::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Generic", "0.1.0"))
        .try_build()
        .expect("generic responses build");
    let document = serde_json::to_value(built.document()).unwrap();

    assert_eq!(
        document["paths"]["/generic/maybe"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/Thing"
    );
    assert_eq!(
        document["paths"]["/generic/maybe"]["get"]["responses"]["404"]["description"],
        "Not Found"
    );

    let paged = &document["paths"]["/generic/paged"]["get"]["responses"]["200"];
    assert_eq!(
        paged["content"]["application/json"]["schema"]["type"],
        "array"
    );
    let header_type = paged["headers"]["x-next-cursor"]["schema"]["type"]
        .as_array()
        .unwrap();
    assert!(header_type.contains(&serde_json::json!("string")));

    assert_eq!(
        document["paths"]["/generic/wrapped"]["get"]["responses"]["500"]["description"],
        "Internal Server Error"
    );
}

#[tokio::test]
async fn generic_response_enums_answer_the_status() {
    let built = roundhouse::App::new(State).include(&routes::MODULE).build();
    let response = built.oneshot_get("/generic/paged").await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let header = response.headers().get("x-next-cursor").unwrap();
    assert_eq!(header, "next");
}
