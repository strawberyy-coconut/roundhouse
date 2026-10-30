//! `#[serde(rename_all = "...")]` must resolve identically for the document and serde.
//!
//! serde deliberately uses different field and variant rules: `lowercase` and `snake_case` are
//! identities for fields, but not for variants. These tests pin the field rules because a query
//! or path derive that guesses differently produces a document the runtime does not accept.

use axum::http::StatusCode;
use roundhouse::App;
use roundhouse::prelude::*;
use serde::Deserialize;

#[derive(Clone, Default)]
pub struct State;

fn document(module: &'static roundhouse::Module) -> serde_json::Value {
    serde_json::to_value(
        App::new(State)
            .include(module)
            .openapi(|| roundhouse::openapi::Info::new("Renames", "0.1.0"))
            .build()
            .document(),
    )
    .unwrap()
}

async fn get(module: &'static roundhouse::Module, uri: &str) -> (StatusCode, String) {
    let app = App::new(State)
        .include(module)
        .openapi(|| roundhouse::openapi::Info::new("Renames", "0.1.0"))
        .build();
    let response = app.oneshot_get(uri).await;
    let status = response.status();
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    (status, String::from_utf8_lossy(&body).to_string())
}

fn query_parameter_names(module: &'static roundhouse::Module, path: &str) -> Vec<String> {
    document(module)["paths"][path]["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|parameter| parameter["name"].as_str().unwrap().to_string())
        .collect()
}

mod query_rules {
    use super::*;

    routes! {
        prefix = "/rename-rules",
        get("/lower", lower, operation_id = "rename_rules_lower"),
        get("/upper", upper, operation_id = "rename_rules_upper"),
        get("/kebab", kebab, operation_id = "rename_rules_kebab"),
        get("/pascal", pascal, operation_id = "rename_rules_pascal"),
    }

    #[derive(Deserialize, QueryParams)]
    #[serde(rename_all = "lowercase")]
    pub struct Lower {
        pub per_page: Option<u32>,
        pub sort_by: Option<String>,
    }

    #[derive(Deserialize, QueryParams)]
    #[serde(rename_all = "UPPERCASE")]
    pub struct Upper {
        pub per_page: Option<u32>,
        pub sort_by: Option<String>,
    }

    #[derive(Deserialize, QueryParams)]
    #[serde(rename_all = "kebab-case")]
    pub struct Kebab {
        pub per_page: Option<u32>,
        pub sort_by: Option<String>,
    }

    #[derive(Deserialize, QueryParams)]
    #[serde(rename_all = "PascalCase")]
    pub struct Pascal {
        pub per_page: Option<u32>,
        pub sort_by: Option<String>,
    }

    fn echo(page: Option<u32>, sort: Option<String>) -> String {
        format!("{}:{}", page.unwrap_or(0), sort.unwrap_or_default())
    }

    #[handler]
    pub async fn lower(Query(query): Query<Lower>) -> String {
        echo(query.per_page, query.sort_by)
    }

    #[handler]
    pub async fn upper(Query(query): Query<Upper>) -> String {
        echo(query.per_page, query.sort_by)
    }

    #[handler]
    pub async fn kebab(Query(query): Query<Kebab>) -> String {
        echo(query.per_page, query.sort_by)
    }

    #[handler]
    pub async fn pascal(Query(query): Query<Pascal>) -> String {
        echo(query.per_page, query.sort_by)
    }
}

#[tokio::test]
async fn field_rename_all_rules_match_serde() {
    let module = &query_rules::MODULE;

    let lower = query_parameter_names(module, "/rename-rules/lower");
    assert_eq!(lower, ["per_page", "sort_by"]);

    let upper = query_parameter_names(module, "/rename-rules/upper");
    assert_eq!(upper, ["PER_PAGE", "SORT_BY"]);

    let kebab = query_parameter_names(module, "/rename-rules/kebab");
    assert_eq!(kebab, ["per-page", "sort-by"]);

    let pascal = query_parameter_names(module, "/rename-rules/pascal");
    assert_eq!(pascal, ["PerPage", "SortBy"]);

    // The document's names are also the names serde accepts at runtime.
    assert_eq!(
        get(module, "/rename-rules/lower?per_page=2&sort_by=name").await,
        (StatusCode::OK, "2:name".to_string())
    );
    assert_eq!(
        get(module, "/rename-rules/upper?PER_PAGE=2&SORT_BY=name").await,
        (StatusCode::OK, "2:name".to_string())
    );
    assert_eq!(
        get(module, "/rename-rules/kebab?per-page=2&sort-by=name").await,
        (StatusCode::OK, "2:name".to_string())
    );
    assert_eq!(
        get(module, "/rename-rules/pascal?PerPage=2&SortBy=name").await,
        (StatusCode::OK, "2:name".to_string())
    );
}

mod path_upper {
    use super::*;

    routes! {
        prefix = "/path-upper",
        get("/{PER_PAGE}", show, operation_id = "path_upper_show"),
    }

    #[derive(Deserialize, PathParams)]
    #[serde(rename_all = "UPPERCASE")]
    pub struct Paging {
        pub per_page: u32,
    }

    #[handler]
    pub async fn show(Path(paging): Path<Paging>) -> String {
        paging.per_page.to_string()
    }
}

#[tokio::test]
async fn path_field_rename_all_matches_serde() {
    let module = &path_upper::MODULE;
    let parameters = document(module)["paths"]["/path-upper/{PER_PAGE}"]["get"]["parameters"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0]["name"], "PER_PAGE");

    assert_eq!(
        get(module, "/path-upper/2").await,
        (StatusCode::OK, "2".to_string())
    );
}
