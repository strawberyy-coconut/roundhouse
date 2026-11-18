//! A production-shaped app compiled without the `contract` feature.
//!
//! The workspace builds this crate so the runtime surface keeps compiling with no document
//! builder and no `utoipa` dependency. `cargo tree -p production-fixture` is the CI check for the
//! dependency graph itself.

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct AppState;

roundhouse::app_state!(AppState);

#[roundhouse::schema]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Note {
    #[roundhouse(min_length = 1, max_length = 120)]
    pub title: String,
}

#[derive(serde::Deserialize, QueryParams)]
pub struct ListQuery {
    pub limit: Option<u32>,
}

roundhouse::routes! {
    prefix = "/v1",
    "/notes" => { get(list), post(create) },
    "/notes/{id}" => { get(show) },
    "/opaque" => { get(opaque) },
    "/conflict" => { get(conflict) },
}

#[handler]
pub async fn list(Query(query): Query<ListQuery>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "limit": query.limit }))
}

#[handler]
pub async fn create(Json(note): Json<Note>) -> Json<Note> {
    Json(note)
}

#[handler]
pub async fn show(Path(id): Path<u64>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "id": id }))
}

#[handler(response(200))]
pub async fn opaque() -> impl IntoResponse {
    Json(Note {
        title: "opaque".to_string(),
    })
}

#[derive(Debug)]
pub struct Conflict;

impl IntoResponse for Conflict {
    fn into_response(self) -> Response {
        (StatusCode::CONFLICT, "conflict").into_response()
    }
}

#[roundhouse::build_only]
impl roundhouse::doc::DocumentedError for Conflict {
    fn responses(
        _context: &mut dyn roundhouse::doc::SchemaContext,
    ) -> Vec<roundhouse::doc::ResponseDoc> {
        vec![roundhouse::doc::ResponseDoc::problem(409, "Conflict")]
    }
}

#[allow(clippy::result_large_err)]
#[handler]
pub async fn conflict() -> Result<Json<Note>, Conflict> {
    Ok(Json(Note {
        title: "conflict".to_string(),
    }))
}

pub fn app() -> roundhouse::Built {
    // No contract artifact ships with this fixture; turn validation off explicitly.
    roundhouse::app!(AppState)
        .validation(roundhouse::Validation::Off)
        .build()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_runtime_builds_routes_without_a_contract() {
        let built = super::app();
        assert_eq!(built.routes().len(), 5);
    }

    #[test]
    fn validation_without_a_contract_is_an_error() {
        // Workspace feature unification can enable `contract` for this fixture's dependency;
        // the runtime-only behavior is what this test covers.
        if roundhouse::CONTRACT_COMPILED {
            return;
        }
        let error = match roundhouse::App::new(super::AppState)
            .fragment(super::__roundhouse_fragment())
            .try_build()
        {
            Ok(_) => panic!("validation must require a contract"),
            Err(error) => error,
        };
        assert!(matches!(error, roundhouse::BuildError::MissingContract));
    }
}
