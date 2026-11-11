//! Result aliases are documented through the `ResultShape` probe.

use roundhouse::prelude::*;
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Thing {
    pub id: u64,
}

#[derive(Debug, thiserror::Error, ApiError)]
pub enum NoteError {
    #[error("not found")]
    #[status(404)]
    NotFound,
}

type ApiResult<T> = Result<T, NoteError>;
mod aliased {
    use super::*;

    /// @responses:
    ///   200
    #[handler]
    pub async fn show() -> ApiResult<Json<Thing>> {
        Ok(Json(Thing { id: 1 }))
    }

    routes! {
        prefix = "/aliased",
        "/" => {
            get(show),
        },
    }
}

#[test]
fn a_result_alias_documents_both_sides() {
    let built = roundhouse::App::new(State)
        .include(&aliased::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Aliases", "0.1.0"))
        .try_build()
        .expect("alias builds");
    let document = serde_json::to_value(built.document()).unwrap();
    let responses = &document["paths"]["/aliased"]["get"]["responses"];
    assert_eq!(
        responses["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/Thing"
    );
    assert_eq!(responses["404"]["description"], "Not Found");
}
