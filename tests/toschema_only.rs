use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct Payload {
    #[schema(min_length = 3)]
    pub name: String,
}

mod more {
    use super::*;

    routes! {
        prefix = "/more",
        post("/created", created, operation_id = "toschema_created"),
        get("/declared", declared, operation_id = "toschema_declared", responses(200 = Payload)),
    }

    #[derive(ApiResponse)]
    pub enum Created {
        #[status(201, description = "Created")]
        Created {
            #[body]
            body: Json<Payload>,
            #[location]
            location: String,
        },
    }

    #[allow(clippy::result_large_err)]
    #[handler]
    pub async fn created(Json(body): Json<Payload>) -> Result<Created, Problem> {
        Ok(Created::Created {
            body: Json(body),
            location: "/t/more/created".to_string(),
        })
    }

    #[handler]
    pub async fn declared() -> axum::http::StatusCode {
        axum::http::StatusCode::NO_CONTENT
    }
}

routes! {
    prefix = "/t",
    post("/x", create, operation_id = "toschema_create"),
    get("/y", get, operation_id = "toschema_get"),
    ...more,
}

#[handler]
pub async fn create(Json(body): Json<Payload>) -> Json<Payload> {
    Json(body)
}

#[handler]
pub async fn get() -> Json<Payload> {
    Json(Payload { name: "abc".into() })
}

#[tokio::test]
async fn toschema_only_body_and_response_work() {
    let built = roundhouse::app!(State).strict().build();
    let doc = serde_json::to_value(built.document()).unwrap();
    assert_eq!(
        doc["paths"]["/t/x"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"]
            ["name"]["minLength"],
        3
    );
    assert_eq!(
        doc["paths"]["/t/y"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["properties"]
            ["name"]["minLength"],
        3
    );
    let resp = built
        .oneshot_json(http::Method::POST, "/t/x", r#"{"name":"ab"}"#)
        .await;
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let resp = built
        .oneshot_json(http::Method::POST, "/t/x", r#"{"name":"abc"}"#)
        .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
}

#[test]
fn to_schema_only_response_enum_and_declared_response_work() {
    let built = roundhouse::app!(State).strict().build();
    let doc = serde_json::to_value(built.document()).unwrap();
    assert_eq!(
        doc["paths"]["/t/more/created"]["post"]["responses"]["201"]["content"]["application/json"]
            ["schema"]["properties"]["name"]["minLength"],
        3
    );
    assert_eq!(
        doc["paths"]["/t/more/declared"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["properties"]["name"]["minLength"],
        3
    );
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema)]
pub struct PlainInner {
    pub value: String,
}

#[derive(serde::Serialize, serde::Deserialize, ToSchema, ApiSchema)]
pub struct HookOuter {
    pub inner: PlainInner,
    pub inners: Vec<PlainInner>,
    #[roundhouse(unique_items)]
    pub tags: Vec<String>,
}

#[test]
fn an_api_schema_container_can_hold_plain_to_schema_fields() {
    let mut components = roundhouse::utoipa::openapi::schema::Components::new();
    <HookOuter as roundhouse::doc::ApiSchema>::register(&mut components);
    assert!(components.schemas.contains_key("PlainInner"));
    assert!(!components.schemas.contains_key("Vec"));
    let outer = serde_json::to_value(&components.schemas["HookOuter"]).unwrap();
    assert_eq!(outer["properties"]["tags"]["uniqueItems"], true);
}
