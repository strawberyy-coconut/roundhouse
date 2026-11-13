//! Typed `#[handler(..)]` metadata produces the same document as doc tags, without a string DSL.

use roundhouse::interop::ToSchema;
use roundhouse::prelude::*;
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

roundhouse::app_state!(State);

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Payload {
    pub value: i64,
}

#[derive(Serialize, ToSchema, ApiSchema)]
pub struct Other {
    pub other: i64,
}

mod typed {
    use super::*;

    routes! {
        "/typed/{id}" => { get(show, operation_id = "typed_show") },
        "/override" => { get(over, operation_id = "typed_override") },
    }

    #[allow(clippy::result_large_err)]
    #[handler(
        response(200, Json<Payload>),
        response(409, Problem),
        response_description(200, "The payload."),
        path(id, "The payload identifier."),
    )]
    pub async fn show(Path(_id): Path<String>) -> Result<Json<Payload>, Problem> {
        Ok(Json(Payload { value: 1 }))
    }

    #[handler(override_response(200, Payload))]
    pub async fn over() -> Json<Other> {
        Json(Other { other: 2 })
    }
}

fn built() -> roundhouse::Built {
    roundhouse::App::new(State)
        .include(&typed::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Handler args", "0.1.0"))
        .build()
}

#[test]
fn typed_responses_and_path_descriptions_reach_the_document() {
    let document = serde_json::to_value(built().document()).unwrap();
    let operation = &document["paths"]["/typed/{id}"]["get"];

    assert_eq!(
        operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/Payload"
    );
    assert_eq!(operation["responses"]["200"]["description"], "The payload.");
    assert!(
        operation["responses"]["409"]["content"]["application/problem+json"]["schema"]["properties"]
            ["title"]
            .is_object()
    );
    let parameter = operation["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["name"] == "id")
        .unwrap();
    assert_eq!(parameter["in"], "path");
    assert_eq!(parameter["description"], "The payload identifier.");
}

#[test]
fn override_response_replaces_the_inferred_shape_and_is_marked() {
    let document = serde_json::to_value(built().document()).unwrap();
    let operation = &document["paths"]["/override"]["get"];

    assert_eq!(
        operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/Payload"
    );
    assert_eq!(operation["x-roundhouse-overridden-response"], true);
}
