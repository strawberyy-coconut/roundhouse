//! Parameter schemas must register their components, and a nullable enum must accept an absent
//! optional value.

use axum::http::StatusCode;
use roundhouse::prelude::*;
use roundhouse::{App, BuildError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct State;

fn app(module: &'static roundhouse::Module) -> roundhouse::Built {
    App::new(State)
        .include(module)
        .openapi(|| roundhouse::openapi::Info::new("Parameters", "0.1.0"))
        .build()
}

fn document(module: &'static roundhouse::Module) -> serde_json::Value {
    serde_json::to_value(app(module).document()).unwrap()
}

fn parameter<'a>(document: &'a serde_json::Value, path: &str, name: &str) -> &'a serde_json::Value {
    document["paths"][path]["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["name"] == name)
        .unwrap_or_else(|| panic!("missing parameter `{name}` in {path}"))
}

mod optional_toschema_enum {
    use super::*;

    routes! {
        prefix = "/optional",
        "/" => { get(list) },
    }

    #[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Order {
        Asc,
        Desc,
    }

    #[derive(Debug, Deserialize, QueryParams)]
    pub struct Filter {
        pub order: Option<Order>,
    }

    #[handler]
    pub async fn list(Query(filter): Query<Filter>) -> String {
        filter
            .order
            .map(|order| format!("{order:?}"))
            .unwrap_or_else(|| "none".to_string())
    }
}

#[tokio::test]
async fn an_absent_optional_toschema_enum_is_valid() {
    let document = document(&optional_toschema_enum::MODULE);
    let order = parameter(&document, "/optional", "order");
    assert_eq!(order["required"], false);
    assert_eq!(
        order["schema"]["enum"],
        serde_json::json!(["asc", "desc", null])
    );

    let built = app(&optional_toschema_enum::MODULE);
    let response = built.oneshot_get("/optional").await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = built.oneshot_get("/optional?order=desc").await;
    assert_eq!(response.status(), StatusCode::OK);
}

mod optional_apischema_enum {
    use super::*;

    routes! {
        prefix = "/apis",
        "/" => { get(list) },
    }

    #[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, ApiSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Sort {
        Asc,
        Desc,
    }

    #[derive(Debug, Deserialize, QueryParams)]
    pub struct Filter {
        pub sort: Option<Sort>,
    }

    #[handler]
    pub async fn list(Query(filter): Query<Filter>) -> String {
        filter
            .sort
            .map(|sort| format!("{sort:?}"))
            .unwrap_or_else(|| "none".to_string())
    }
}

#[tokio::test]
async fn a_query_field_that_derives_apischema_registers_its_component() {
    let document = document(&optional_apischema_enum::MODULE);
    assert!(document["components"]["schemas"]["Sort"].is_object());
    assert_eq!(
        parameter(&document, "/apis", "sort")["schema"]["oneOf"][0]["$ref"],
        "#/components/schemas/Sort"
    );

    let built = app(&optional_apischema_enum::MODULE);
    let response = built.oneshot_get("/apis").await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = built.oneshot_get("/apis?sort=desc").await;
    assert_eq!(response.status(), StatusCode::OK);
}

mod repeated_apischema_enum {
    use super::*;

    routes! {
        prefix = "/repeated",
        "/" => { get(list) },
    }

    #[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, ApiSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Sort {
        Asc,
        Desc,
    }

    #[derive(Debug, Deserialize, QueryParams)]
    pub struct Filter {
        pub sorts: Vec<Sort>,
    }

    #[handler]
    pub async fn list(RepeatedQuery(filter): RepeatedQuery<Filter>) -> String {
        format!("{}", filter.sorts.len())
    }
}

#[tokio::test]
async fn a_repeated_query_field_registers_its_item_component() {
    let document = document(&repeated_apischema_enum::MODULE);
    assert!(
        document["components"]["schemas"]["Sort"].is_object(),
        "the repeated item must be registered: {document}"
    );

    let built = app(&repeated_apischema_enum::MODULE);
    let response = built.oneshot_get("/repeated?sorts=asc&sorts=desc").await;
    assert_eq!(response.status(), StatusCode::OK);
}

mod path_apischema_enum {
    use super::*;

    routes! {
        prefix = "/path-enum",
        "/{sort}" => { get(show) },
    }

    #[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, ApiSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Sort {
        Asc,
        Desc,
    }

    #[derive(Debug, Deserialize, PathParams)]
    pub struct PathArgs {
        pub sort: Sort,
    }

    #[handler]
    pub async fn show(Path(args): Path<PathArgs>) -> String {
        format!("{:?}", args.sort)
    }
}

#[tokio::test]
async fn a_path_param_struct_registers_its_field_components() {
    let document = document(&path_apischema_enum::MODULE);
    assert!(
        document["components"]["schemas"]["Sort"].is_object(),
        "the path field must be registered: {document}"
    );

    let built = app(&path_apischema_enum::MODULE);
    let response = built.oneshot_get("/path-enum/asc").await;
    assert_eq!(response.status(), StatusCode::OK);
}

mod flattened_apischema_enum {
    use super::*;

    routes! {
        prefix = "/flattened",
        "/" => { get(list) },
    }

    #[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, ApiSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum Sort {
        Asc,
        Desc,
    }

    #[derive(Debug, Deserialize, QueryParams)]
    pub struct Inner {
        pub sort: Option<Sort>,
    }

    #[derive(Debug, Deserialize, QueryParams)]
    pub struct Filter {
        #[serde(flatten)]
        pub inner: Inner,
    }

    #[handler]
    pub async fn list(Query(_filter): Query<Filter>) -> String {
        "ok".to_string()
    }
}

#[tokio::test]
async fn a_flattened_query_struct_registers_its_field_components() {
    let document = document(&flattened_apischema_enum::MODULE);
    assert!(
        document["components"]["schemas"]["Sort"].is_object(),
        "the flattened field must be registered: {document}"
    );
}

mod dangling_reference {
    use super::*;
    use roundhouse::doc::{Param, ParamIn};

    routes! {
        prefix = "/dangling",
        "/" => {
            get(list, doc = |docs| {
                docs.push_param(Param::new(
                    "missing",
                    ParamIn::Query,
                    roundhouse::doc::schema::r#ref("Missing"),
                ));
            }),
        },
    }

    #[handler]
    pub async fn list() -> &'static str {
        "ok"
    }
}

#[test]
fn a_dangling_component_reference_is_a_document_error() {
    let error = App::new(State)
        .include(&dangling_reference::MODULE)
        .try_build()
        .err()
        .expect("a dangling reference must not build");
    assert!(matches!(error, BuildError::InvalidDocument { .. }));
    assert!(error.to_string().contains("has no component `Missing`"));
}
