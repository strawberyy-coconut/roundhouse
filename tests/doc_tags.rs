//! Compatibility doc tags: prose and the structured tags that typed metadata replaced.

use roundhouse::interop::ToSchema;
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

mod things {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// @description: A stored thing.
    #[derive(Serialize, ToSchema, ApiSchema)]
    pub struct Thing {
        pub id: u64,
    }

    #[derive(Deserialize, QueryParams)]
    pub struct Filter {
        /// @description: Free-text search across things.
        pub q: Option<String>,
        #[schema(description = "Maximum number of results.")]
        pub limit: Option<u32>,
    }

    #[derive(Deserialize, PathParams)]
    pub struct ThingId {
        /// @description: The thing's identifier.
        pub id: u64,
    }

    /// @summary: List things.
    /// @description: Returns every thing visible to the caller.
    #[handler]
    pub async fn list(Query(_filter): Query<Filter>) -> Json<Vec<Thing>> {
        Json(Vec::new())
    }

    /// @summary: Read one thing.
    #[handler]
    pub async fn show(Path(ThingId { id }): Path<ThingId>) -> Json<Thing> {
        Json(Thing { id })
    }

    /// @summary: Create a thing.
    #[handler]
    pub async fn create() -> CreatedOrMissing {
        CreatedOrMissing::Created(Json(Thing { id: 1 }))
    }

    /// @summary: Delete a thing.
    #[handler(response(204))]
    pub async fn destroy() -> axum::http::StatusCode {
        axum::http::StatusCode::NO_CONTENT
    }

    #[allow(dead_code)]
    #[derive(ApiResponse)]
    pub enum CreatedOrMissing {
        /// @description: The thing was created.
        #[status(201)]
        Created(Json<Thing>),

        /// @description: The thing was missing.
        #[status(404)]
        Missing,
    }

    routes! {
        prefix = "/things",
        "/" => {
            get(list),
            post(create),
        },
        "/{id}" => {
            get(show),
        },
        "/delete" => {
            delete(destroy),
        },
    }
}

mod declared {
    use super::*;

    /// @summary: Declare responses without an enum.
    #[handler(response(201, Thing))]
    pub async fn create() -> Json<Thing> {
        Json(Thing { id: 1 })
    }

    #[handler(response(204))]
    pub async fn destroy() -> axum::http::StatusCode {
        axum::http::StatusCode::NO_CONTENT
    }

    routes! {
        prefix = "/declared",
        "/" => {
            post(create),
        },
        "/none" => {
            delete(destroy),
        },
    }

    use super::things::Thing;
}

mod hooked {
    use super::*;
    use crate::things::Thing;

    /// @summary: The hook owns the prose.
    #[handler(response(200, Thing))]
    pub async fn show() -> Json<Thing> {
        Json(Thing { id: 1 })
    }

    routes! {
        prefix = "/hooked",
        "/" => {
            get(show, doc = |docs| {
                if let Some(response) = docs.response_mut(200) {
                    response.description = "From the doc hook".to_string();
                }
            }),
        },
    }
}

mod problem_response {
    use super::*;

    #[handler(response(400, Problem))]
    pub async fn bad() -> axum::http::StatusCode {
        axum::http::StatusCode::BAD_REQUEST
    }

    routes! {
        prefix = "/problem",
        "/" => {
            get(bad),
        },
    }
}

mod summary_boundary {
    use super::things::Thing;
    use super::*;

    /// @summary: A short summary.
    ///
    /// This rustdoc paragraph is for readers, not for OpenAPI.
    #[handler]
    pub async fn show() -> Json<Thing> {
        Json(Thing { id: 1 })
    }

    routes! {
        prefix = "/summary",
        "/" => { get(show) },
    }
}

mod scalar_path {
    use super::*;

    /// @summary: A scalar path.
    #[handler(path(id, "The scalar identifier."))]
    pub async fn show(Path(id): Path<String>) -> String {
        id
    }

    routes! {
        prefix = "/scalar",
        "/{id}" => { get(show) },
    }
}

mod path_precedence {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, PathParams)]
    pub struct Args {
        /// @description: The field description.
        pub id: i64,
    }

    #[handler(path(id, "The handler description."))]
    pub async fn show(Path(args): Path<Args>) -> String {
        args.id.to_string()
    }

    routes! {
        prefix = "/precedence",
        "/{id}" => { get(show) },
    }
}

mod unknown_path {
    use super::*;

    #[handler(path(missing, "No such capture."))]
    pub async fn show() -> &'static str {
        "x"
    }

    routes! {
        prefix = "/unknown",
        "/x" => { get(show) },
    }
}

fn document(module: &'static roundhouse::Module) -> serde_json::Value {
    let built = roundhouse::App::new(State)
        .include(module)
        .openapi(|| roundhouse::openapi::Info::new("Doc tags", "0.1.0"))
        .build();
    serde_json::to_value(built.document()).unwrap()
}

#[test]
fn handler_tags_fill_summary_and_description() {
    let document = document(&things::MODULE);
    let operation = &document["paths"]["/things"]["get"];
    assert_eq!(operation["summary"], "List things.");
    assert_eq!(
        operation["description"],
        "Returns every thing visible to the caller."
    );
}

#[test]
fn query_and_path_parameter_tags_fill_descriptions() {
    let document = document(&things::MODULE);
    let query = &document["paths"]["/things"]["get"]["parameters"];
    let q = query
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["name"] == "q")
        .unwrap();
    assert_eq!(q["description"], "Free-text search across things.");
    let limit = query
        .as_array()
        .unwrap()
        .iter()
        .find(|param| param["name"] == "limit")
        .unwrap();
    assert_eq!(limit["description"], "Maximum number of results.");

    let path = &document["paths"]["/things/{id}"]["get"]["parameters"][0];
    assert_eq!(path["name"], "id");
    assert_eq!(path["description"], "The thing's identifier.");
}

#[test]
fn response_enum_variants_describe_themselves() {
    let document = document(&things::MODULE);
    let responses = &document["paths"]["/things"]["post"]["responses"];
    assert_eq!(responses["201"]["description"], "The thing was created.");
    assert_eq!(responses["404"]["description"], "The thing was missing.");
}

#[test]
fn a_single_body_response_uses_the_type_description() {
    let document = document(&things::MODULE);
    assert_eq!(
        document["paths"]["/things/{id}"]["get"]["responses"]["200"]["description"],
        "A stored thing."
    );
}

#[test]
fn declared_responses_accept_json_wrappers_and_empty_statuses() {
    let document = document(&declared::MODULE);
    let responses = &document["paths"]["/declared"]["post"]["responses"];
    assert_eq!(
        responses["201"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/Thing"
    );
    assert_eq!(responses["201"]["description"], "A stored thing.");

    let empty = &document["paths"]["/declared/none"]["delete"]["responses"]["204"];
    assert_eq!(empty["description"], "No Content");
    assert!(empty.get("content").is_none());
}

#[test]
fn a_doc_hook_description_survives_the_declared_response_merge() {
    let document = document(&hooked::MODULE);
    assert_eq!(
        document["paths"]["/hooked"]["get"]["responses"]["200"]["description"],
        "From the doc hook"
    );
}

#[test]
fn declared_problem_responses_use_the_problem_schema() {
    let document = document(&problem_response::MODULE);
    let response = &document["paths"]["/problem"]["get"]["responses"]["400"];
    assert!(response["content"]["application/problem+json"]["schema"].is_object());
    assert_eq!(response["description"], "Bad Request");
}

#[test]
fn a_summary_stops_at_a_blank_line() {
    let document = document(&summary_boundary::MODULE);
    assert_eq!(
        document["paths"]["/summary"]["get"]["summary"],
        "A short summary."
    );
    assert!(
        document["paths"]["/summary"]["get"]
            .get("description")
            .is_none()
    );
}

#[test]
fn a_scalar_path_can_carry_a_handler_description() {
    let document = document(&scalar_path::MODULE);
    let parameter = &document["paths"]["/scalar/{id}"]["get"]["parameters"][0];
    assert_eq!(parameter["name"], "id");
    assert_eq!(parameter["description"], "The scalar identifier.");
}

#[test]
fn a_path_params_field_description_wins_over_the_handler_tag() {
    let document = document(&path_precedence::MODULE);
    let parameter = &document["paths"]["/precedence/{id}"]["get"]["parameters"][0];
    assert_eq!(parameter["description"], "The field description.");
}

mod described {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize, ToSchema, ApiSchema)]
    #[schema(description = "A schema-described thing.")]
    pub struct SchemaThing {
        pub id: u64,
    }

    #[allow(dead_code)]
    #[derive(ApiResponse)]
    pub enum Outcome {
        #[status(200)]
        Found(Json<SchemaThing>),
        #[status(404)]
        Missing,
    }

    /// @summary: Use the schema description.
    #[handler]
    pub async fn found() -> Outcome {
        Outcome::Found(Json(SchemaThing { id: 1 }))
    }

    #[handler(response_description(200, "A route-specific description."))]
    pub async fn overridden() -> Json<SchemaThing> {
        Json(SchemaThing { id: 1 })
    }

    routes! {
        prefix = "/described",
        "/found" => { get(found) },
        "/overridden" => { get(overridden) },
    }
}

#[test]
fn collection_responses_inherit_the_element_description() {
    let document = document(&things::MODULE);
    assert_eq!(
        document["paths"]["/things"]["get"]["responses"]["200"]["description"],
        "A stored thing."
    );
}

#[test]
fn an_api_response_variant_inherits_the_body_type_description() {
    let document = document(&described::MODULE);
    assert_eq!(
        document["paths"]["/described/found"]["get"]["responses"]["200"]["description"],
        "A schema-described thing."
    );
    assert_eq!(
        document["paths"]["/described/found"]["get"]["responses"]["404"]["description"],
        "Not Found"
    );
}

#[test]
fn a_response_description_tag_overrides_the_body_type() {
    let document = document(&described::MODULE);
    assert_eq!(
        document["paths"]["/described/overridden"]["get"]["responses"]["200"]["description"],
        "A route-specific description."
    );
}

#[test]
fn a_path_description_without_a_capture_is_a_build_error() {
    let error = roundhouse::App::new(State)
        .include(&unknown_path::MODULE)
        .try_build()
        .err()
        .expect("an unknown path name must not build");
    assert!(matches!(
        error,
        roundhouse::BuildError::UnknownPathParameter { .. }
    ));
}

roundhouse::app_state!(State);
