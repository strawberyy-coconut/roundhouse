//! The opt-in adapter for generated `ToSchema` types.

use roundhouse::prelude::*;
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

#[derive(Serialize, ToSchema)]
pub struct Row {
    pub id: u64,
    pub nested: Nested,
}

#[derive(Serialize, ToSchema)]
pub struct Nested {
    pub name: String,
}

roundhouse::impl_api_schema!(Row, Nested);

mod routes {
    use super::*;

    routes! {
        prefix = "/generated",
        "/" => { get(show) },
    }

    #[handler]
    pub async fn show() -> Json<Row> {
        Json(Row {
            id: 1,
            nested: Nested {
                name: "nested".to_string(),
            },
        })
    }
}

#[test]
fn a_toschema_only_generated_type_can_be_documented() {
    let built = roundhouse::App::new(State)
        .include(&routes::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Generated", "0.1.0"))
        .try_build()
        .expect("the adapter registers the generated type");
    let document = serde_json::to_value(built.document()).unwrap();
    assert!(document["components"]["schemas"]["Row"].is_object());
    assert!(document["components"]["schemas"]["Nested"].is_object());
    assert_eq!(
        document["paths"]["/generated"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/Row"
    );
}

roundhouse::app_state!(State);
