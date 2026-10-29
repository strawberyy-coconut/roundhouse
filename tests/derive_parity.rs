//! Derive parity: what serde accepts is what the document says.
//!
//! These tests exercise the shared serde-shape parser from the query and path derives, plus the
//! extractor contracts the document promises (`style: form, explode: true` for arrays, and the
//! unverified-extractor marker).

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use roundhouse::prelude::*;
use roundhouse::{App, BuildError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct State;

fn app(module: &'static str) -> roundhouse::Built {
    App::new(State)
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Parity", "0.1.0"))
        .build()
}

fn document(module: &'static str) -> serde_json::Value {
    serde_json::to_value(app(module).document().unwrap()).unwrap()
}

async fn get(module: &'static str, uri: &str) -> (StatusCode, String) {
    let app = app(module);
    let response = app.oneshot_get(uri).await;
    let status = response.status();
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

fn query_parameters(document: &serde_json::Value, path: &str) -> Vec<serde_json::Value> {
    document["paths"][path]["get"]["parameters"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn parameter<'a>(parameters: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    parameters
        .iter()
        .find(|parameter| parameter["name"] == name)
        .unwrap_or_else(|| panic!("missing parameter `{name}` in {parameters:?}"))
}

mod query_names {
    use super::*;

    mount!("/query-names");

    routes! { get("/list", list, operation_id = "query_names_list") }

    #[derive(Deserialize, QueryParams)]
    #[serde(rename_all = "camelCase")]
    pub struct ListQuery {
        pub per_page: Option<u32>,
        #[serde(default)]
        pub include_archived: bool,
    }

    #[handler]
    pub async fn list(Query(query): Query<ListQuery>) -> String {
        format!("{}:{}", query.per_page.unwrap_or(0), query.include_archived)
    }
}

#[tokio::test]
async fn query_names_follow_serde_rename_all_and_default() {
    let document = document(concat!(module_path!(), "::query_names"));
    let parameters = query_parameters(&document, "/query-names/list");
    assert!(parameters.iter().any(|p| p["name"] == "perPage"));
    assert!(parameters.iter().any(|p| p["name"] == "includeArchived"));
    assert!(!parameters.iter().any(|p| p["name"] == "per_page"));
    // `includeArchived` has a field-level default; it is optional in the document too.
    assert_eq!(parameter(&parameters, "perPage")["required"], false);
    assert_eq!(parameter(&parameters, "includeArchived")["required"], false);

    let (status, body) = get(
        concat!(module_path!(), "::query_names"),
        "/query-names/list?perPage=5&includeArchived=true",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "5:true");

    // The snake_case spelling is not the wire name: serde ignores it.
    let (status, body) = get(
        concat!(module_path!(), "::query_names"),
        "/query-names/list?per_page=5",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "0:false");
}

mod query_arrays {
    use super::*;

    mount!("/query-arrays");

    routes! { get("/filter", filter, operation_id = "query_arrays_filter") }

    #[derive(Deserialize, QueryParams)]
    pub struct Filter {
        pub tags: Vec<String>,
        #[serde(default)]
        pub labels: Vec<String>,
        pub owner: Option<String>,
    }

    #[handler]
    pub async fn filter(Query(query): Query<Filter>) -> String {
        format!(
            "{}|{}|{}",
            query.tags.join(","),
            query.labels.join(","),
            query.owner.unwrap_or_default()
        )
    }
}

#[tokio::test]
async fn repeated_query_keys_are_parsed_and_documented_as_exploded_arrays() {
    let document = document(concat!(module_path!(), "::query_arrays"));
    let parameters = query_parameters(&document, "/query-arrays/filter");
    let tags = parameter(&parameters, "tags");
    assert_eq!(tags["schema"]["type"], "array");
    assert_eq!(tags["style"], "form");
    assert_eq!(tags["explode"], true);
    assert_eq!(tags["required"], true);
    assert_eq!(parameter(&parameters, "labels")["required"], false);

    let (status, body) = get(
        concat!(module_path!(), "::query_arrays"),
        "/query-arrays/filter?tags=a&tags=b",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "a,b||");

    let (status, body) = get(
        concat!(module_path!(), "::query_arrays"),
        "/query-arrays/filter?tags=a&tags=b&labels=x&owner=me",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "a,b|x|me");
}

mod query_container_default {
    use super::*;

    mount!("/query-default");

    routes! { get("/page", page, operation_id = "query_default_page") }

    #[derive(Deserialize, QueryParams)]
    #[serde(default)]
    pub struct Paging {
        pub page: u32,
        pub size: u32,
    }

    impl Default for Paging {
        fn default() -> Self {
            Self { page: 1, size: 10 }
        }
    }

    #[handler]
    pub async fn page(Query(query): Query<Paging>) -> String {
        format!("{}:{}", query.page, query.size)
    }
}

#[tokio::test]
async fn container_default_makes_every_query_field_optional() {
    let document = document(concat!(module_path!(), "::query_container_default"));
    let parameters = query_parameters(&document, "/query-default/page");
    assert_eq!(parameter(&parameters, "page")["required"], false);
    assert_eq!(parameter(&parameters, "size")["required"], false);

    let (status, body) = get(
        concat!(module_path!(), "::query_container_default"),
        "/query-default/page",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "1:10");
}

mod query_skip {
    use super::*;

    mount!("/query-skip");

    routes! { get("/list", list, operation_id = "query_skip_list") }

    #[derive(Deserialize, QueryParams)]
    pub struct Skipping {
        pub page: Option<u32>,
        #[serde(skip)]
        pub internal: Option<String>,
    }

    #[handler]
    pub async fn list(Query(query): Query<Skipping>) -> String {
        format!("{}:{}", query.page.unwrap_or(0), query.internal.is_some())
    }
}

#[tokio::test]
async fn skipped_query_fields_are_not_documented() {
    let document = document(concat!(module_path!(), "::query_skip"));
    let parameters = query_parameters(&document, "/query-skip/list");
    assert!(parameters.iter().all(|p| p["name"] != "internal"));

    let (status, body) = get(
        concat!(module_path!(), "::query_skip"),
        "/query-skip/list?page=2&internal=secret",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "2:false");
}

mod query_flatten {
    use super::*;

    mount!("/query-flatten");

    routes! { get("/list", list, operation_id = "query_flatten_list") }

    #[derive(Deserialize, QueryParams)]
    pub struct Filter {
        pub status: Option<String>,
        pub assignee: Option<String>,
    }

    #[derive(Deserialize, QueryParams)]
    pub struct ListQuery {
        #[serde(flatten)]
        pub filter: Filter,
        pub page: Option<u32>,
    }

    #[handler]
    pub async fn list(Query(query): Query<ListQuery>) -> String {
        format!(
            "{}:{}:{}",
            query.filter.status.unwrap_or_default(),
            query.filter.assignee.unwrap_or_default(),
            query.page.unwrap_or(0)
        )
    }
}

#[tokio::test]
async fn flattened_query_fields_are_top_level_parameters() {
    let document = document(concat!(module_path!(), "::query_flatten"));
    let parameters = query_parameters(&document, "/query-flatten/list");
    let names: Vec<&str> = parameters
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["status", "assignee", "page"]);
    assert!(!names.contains(&"filter"));

    let (status, body) = get(
        concat!(module_path!(), "::query_flatten"),
        "/query-flatten/list?status=open&assignee=me&page=2",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "open:me:2");
}

mod path_names {
    use super::*;

    mount!("/path-names");

    routes! { get("/{projectId}", show, operation_id = "path_names_show") }

    #[derive(Deserialize, PathParams)]
    #[serde(rename_all = "camelCase")]
    pub struct ProjectPath {
        pub project_id: u64,
    }

    #[handler]
    pub async fn show(Path(path): Path<ProjectPath>) -> String {
        path.project_id.to_string()
    }
}

#[tokio::test]
async fn path_names_follow_serde_rename_all() {
    let document = document(concat!(module_path!(), "::path_names"));
    let parameters = document["paths"]["/path-names/{projectId}"]["get"]["parameters"]
        .as_array()
        .unwrap();
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0]["name"], "projectId");

    let (status, body) = get(concat!(module_path!(), "::path_names"), "/path-names/7").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "7");
}

#[cfg(feature = "uuid")]
mod path_uuid {
    use super::*;

    mount!("/path-uuid");

    routes! { get("/{id}", show, operation_id = "path_uuid_show") }

    #[handler]
    pub async fn show(Path(id): Path<uuid::Uuid>) -> String {
        id.to_string()
    }
}

#[cfg(feature = "uuid")]
#[tokio::test]
async fn path_uuid_compiles_and_documents_the_template_capture() {
    let document = document(concat!(module_path!(), "::path_uuid"));
    let parameters = document["paths"]["/path-uuid/{id}"]["get"]["parameters"]
        .as_array()
        .unwrap();
    assert_eq!(parameters[0]["name"], "id");
    assert_eq!(parameters[0]["schema"]["format"], "uuid");

    let id = uuid::Uuid::from_u128(42);
    let (status, body) = get(
        concat!(module_path!(), "::path_uuid"),
        &format!("/path-uuid/{id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, id.to_string());

    let (status, _) = get(
        concat!(module_path!(), "::path_uuid"),
        "/path-uuid/not-a-uuid",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

mod body_container_default {
    use super::*;

    mount!("/body-default");

    routes! { post("/", create, operation_id = "body_default_create") }

    #[derive(Serialize, Deserialize, ApiSchema)]
    #[serde(default)]
    pub struct Body {
        pub name: String,
        pub count: u32,
    }

    impl Default for Body {
        fn default() -> Self {
            Self {
                name: "default".to_string(),
                count: 1,
            }
        }
    }

    #[handler]
    pub async fn create(Json(body): Json<Body>) -> Json<Body> {
        Json(body)
    }
}

#[tokio::test]
async fn a_body_container_default_is_not_required_in_the_schema() {
    let document = document(concat!(module_path!(), "::body_container_default"));
    let schema = &document["components"]["schemas"]["Body"];
    assert!(schema["required"].is_null() || schema["required"].as_array().unwrap().is_empty());

    let app = app(concat!(module_path!(), "::body_container_default"));
    let response = app
        .oneshot_json(http::Method::POST, "/body-default", "{}")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], br#"{"name":"default","count":1}"#);
}

mod custom_extractor {
    use super::*;

    mount!("/tenant");

    routes! { get("/", list, operation_id = "tenant_list") }

    pub struct Tenant(pub String);

    impl<S> FromRequestParts<S> for Tenant
    where
        S: Send + Sync,
    {
        type Rejection = Problem;

        async fn from_request_parts(
            _parts: &mut Parts,
            _state: &S,
        ) -> Result<Self, Self::Rejection> {
            Ok(Tenant("acme".to_string()))
        }
    }

    #[handler]
    pub async fn list(Tenant(tenant): Tenant) -> String {
        tenant
    }
}

#[test]
fn a_custom_extractor_is_marked_and_strict_makes_it_a_diagnostic() {
    let document = document(concat!(module_path!(), "::custom_extractor"));
    assert_eq!(
        document["paths"]["/tenant"]["get"]["x-roundhouse-unverified-extractor"],
        true
    );

    let error = App::new(State)
        .scope(concat!(module_path!(), "::custom_extractor"))
        .openapi(|| roundhouse::openapi::Info::new("Parity", "0.1.0"))
        .strict()
        .try_build()
        .err()
        .expect("strict turns the marker into an error");
    assert!(matches!(error, BuildError::Diagnostic { .. }));
}

mod foreign_extension {
    use super::*;

    mount!("/foreign-extension");

    routes! { get("/", x, operation_id = "foreign_extension_x") }

    #[handler]
    pub async fn x(_value: axum::extract::Extension<std::sync::Arc<usize>>) -> String {
        "x".to_string()
    }
}

#[test]
fn a_foreign_extension_is_marked_rather_than_documented_as_a_problem() {
    let document = document(concat!(module_path!(), "::foreign_extension"));
    let operation = &document["paths"]["/foreign-extension"]["get"];
    assert_eq!(operation["x-roundhouse-unverified-extractor"], true);
    assert!(operation["responses"]["500"].is_null());
}
