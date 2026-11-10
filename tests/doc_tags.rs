//! Explicit doc tags: operation prose, response declarations and parameter descriptions.

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
    /// @responses:
    ///   204
    #[handler]
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
    /// @responses:
    ///   201: Json<Thing>
    #[handler]
    pub async fn create() -> Json<Thing> {
        Json(Thing { id: 1 })
    }

    /// @responses:
    ///   204
    #[handler]
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
    /// @responses:
    ///   200: Thing
    #[handler]
    pub async fn show() -> Json<Thing> {
        Json(Thing { id: 1 })
    }

    routes! {
        prefix = "/hooked",
        "/" => {
            get(show, doc = |docs| {
                for response in &mut docs.responses {
                    if response.status == 200 {
                        response.description = "From the doc hook".to_string();
                    }
                }
            }),
        },
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
fn at_responses_accepts_json_wrappers_and_empty_statuses() {
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
