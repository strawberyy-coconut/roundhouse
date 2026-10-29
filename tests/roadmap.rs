//! The manual-documentation, foreign-schema, merge and mount contracts.

use roundhouse::prelude::*;
use roundhouse::{App, BuildError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct State;

fn document(built: &roundhouse::Built) -> serde_json::Value {
    serde_json::to_value(built.document()).unwrap()
}

mod manual {
    use super::*;

    mount!("/manual");

    routes! {
        get(
            "/items/{id}",
            show,
            operation_id = "manual_show",
            doc = |docs| {
                docs.unverified_extractor = false;
                docs.params.push(
                    roundhouse::Param::new(
                        "trace",
                        roundhouse::ParamIn::Query,
                        roundhouse::doc::schema::string(),
                    )
                    .description("written by hand"),
                );
            },
        ),
    }

    #[handler]
    pub async fn show(
        _value: axum::extract::Extension<std::sync::Arc<usize>>,
        Path(id): Path<u64>,
    ) -> &'static str {
        let _ = id;
        "shown"
    }
}

#[derive(Debug, Serialize, Deserialize, roundhouse::utoipa::ToSchema)]
pub struct Foreign {
    pub id: i64,
}

mod foreign {
    use super::*;

    mount!("/foreign-schema");

    routes! {
        get("/response", response, operation_id = "foreign_schema_response"),
        post("/request", request, operation_id = "foreign_schema_request"),
    }

    #[handler]
    pub async fn response() -> axum::Json<Foreign> {
        axum::Json(Foreign { id: 1 })
    }

    #[handler]
    pub async fn request(body: axum::Json<Foreign>) -> axum::Json<Foreign> {
        body
    }
}

mod mounted {
    use super::*;

    mount!("/inner");

    routes! { get("/x/{id}", x) }

    #[handler]
    pub async fn x(Path(id): Path<u64>) -> String {
        id.to_string()
    }
}

#[test]
fn a_doc_hook_clears_the_marker_and_its_parameter_is_checked() {
    let built = App::new(State)
        .scope(concat!(module_path!(), "::manual"))
        .openapi(|| roundhouse::openapi::Info::new("Roadmap", "0.1.0"))
        .try_build()
        .expect("manual docs build");
    let document = document(&built);
    let operation = &document["paths"]["/manual/items/{id}"]["get"];
    assert!(operation.get("x-roundhouse-unverified-extractor").is_none());
    let parameters = operation["parameters"].as_array().unwrap();
    let trace = parameters
        .iter()
        .find(|parameter| parameter["name"] == "trace")
        .expect("hand-written query parameter");
    assert_eq!(trace["in"], "query");
    assert_eq!(trace["description"], "written by hand");
    assert_eq!(trace["schema"]["type"], "string");
}

mod manual_conflict {
    use super::*;

    mount!("/manual-conflict");

    routes! {
        get(
            "/items/{id}",
            show,
            operation_id = "manual_conflict_show",
            doc = |docs| {
                docs.params.push(roundhouse::Param::new(
                    "id",
                    roundhouse::ParamIn::Path,
                    roundhouse::doc::schema::string(),
                ));
                docs.params.push(roundhouse::Param::new(
                    "id",
                    roundhouse::ParamIn::Path,
                    roundhouse::doc::schema::integer(
                        roundhouse::utoipa::openapi::schema::KnownFormat::Int64,
                    ),
                ));
            },
        ),
    }

    #[handler]
    pub async fn show(Path(id): Path<u64>) -> String {
        id.to_string()
    }
}

#[test]
fn a_hand_written_parameter_goes_through_the_conflict_check() {
    let error = App::new(State)
        .scope(concat!(module_path!(), "::manual_conflict"))
        .openapi(|| roundhouse::openapi::Info::new("Roadmap", "0.1.0"))
        .try_build()
        .err()
        .expect("conflicting hand-written parameters must fail");
    assert!(matches!(error, BuildError::ConflictingParameter { .. }));
}

mod manual_unknown {
    use super::*;

    mount!("/manual-unknown");

    routes! {
        get(
            "/items",
            show,
            operation_id = "manual_unknown_show",
            doc = |docs| {
                docs.params.push(roundhouse::Param::new(
                    "ghost",
                    roundhouse::ParamIn::Path,
                    roundhouse::doc::schema::string(),
                ));
            },
        ),
    }

    #[handler]
    pub async fn show() -> &'static str {
        "shown"
    }
}

#[test]
fn a_hand_written_capture_still_has_to_exist() {
    let error = App::new(State)
        .scope(concat!(module_path!(), "::manual_unknown"))
        .openapi(|| roundhouse::openapi::Info::new("Roadmap", "0.1.0"))
        .try_build()
        .err()
        .expect("an unknown capture must fail");
    assert!(matches!(error, BuildError::UnknownPathParameter { .. }));
}

#[test]
fn a_foreign_utoipa_type_is_documented_but_still_unverified() {
    let built = App::new(State)
        .scope(concat!(module_path!(), "::foreign"))
        .openapi(|| roundhouse::openapi::Info::new("Roadmap", "0.1.0"))
        .try_build()
        .expect("foreign ToSchema types build");
    let document = document(&built);
    assert_eq!(
        document["paths"]["/foreign-schema/response"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/Foreign"
    );
    assert_eq!(
        document["paths"]["/foreign-schema/request"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/Foreign"
    );
    assert!(document["components"]["schemas"]["Foreign"].is_object());
    assert_eq!(
        document["paths"]["/foreign-schema/request"]["post"]["x-roundhouse-unverified-body"],
        true
    );
    assert_eq!(
        document["paths"]["/foreign-schema/response"]["get"]["x-roundhouse-unverified-body"],
        true
    );
}

#[tokio::test]
async fn app_mount_serves_a_scope_at_extra_prefixes() {
    let built = App::new(State)
        .scope(concat!(module_path!(), "::mounted"))
        .mount("/v1", concat!(module_path!(), "::mounted"))
        .mount("/v2", concat!(module_path!(), "::mounted"))
        .openapi(|| roundhouse::openapi::Info::new("Roadmap", "0.1.0"))
        .build();

    for path in ["/inner/x/7", "/v1/inner/x/7", "/v2/inner/x/7"] {
        assert_eq!(
            built.oneshot_get(path).await.status(),
            axum::http::StatusCode::OK,
            "{path}"
        );
    }
    let document = document(&built);
    assert!(document["paths"]["/v1/inner/x/{id}"]["get"].is_object());
    assert!(document["paths"]["/v2/inner/x/{id}"]["get"].is_object());
}

#[test]
fn app_merge_is_visible_and_strict_rejects_it() {
    let merged = axum::Router::new().route("/merged", axum::routing::get(|| async { "merged" }));
    let built = App::new(State)
        .scope(concat!(module_path!(), "::mounted"))
        .merge(merged)
        .openapi(|| roundhouse::openapi::Info::new("Roadmap", "0.1.0"))
        .build();
    let document = document(&built);
    assert_eq!(document["x-roundhouse-merged"], true);
    assert!(
        built
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("App::merge"))
    );
    assert_eq!(built.merges().len(), 1);
    assert_eq!(built.merges()[0].source().module_path, "App::merge");
    assert!(built.merges()[0].location().contains("roadmap.rs"));

    let strict = App::new(State)
        .scope(concat!(module_path!(), "::mounted"))
        .merge(axum::Router::new().route("/merged", axum::routing::get(|| async { "merged" })))
        .strict()
        .try_build()
        .err()
        .expect("strict mode rejects an undocumented merged router");
    assert!(matches!(strict, BuildError::Diagnostic { .. }));
}

#[test]
fn the_facade_covers_info_openapi_and_components() {
    let info = roundhouse::openapi::Info::new("Facade", "1.0.0").description("desc");
    assert_eq!(info.as_utoipa().title, "Facade");
    assert_eq!(info.as_utoipa().description.as_deref(), Some("desc"));

    let _components = roundhouse::openapi::Components::new();
    let built = App::new(State)
        .scope(concat!(module_path!(), "::mounted"))
        .openapi(move || info.clone())
        .build();
    let document: &roundhouse::openapi::OpenApi = built.document();
    assert_eq!(document.as_utoipa().info.title, "Facade");
}

#[tokio::test]
async fn a_mount_handle_replaces_the_string_module_path() {
    let built = App::new(State)
        .scope(mounted::MOUNT)
        .mount("/v3", mounted::MOUNT)
        .openapi(|| roundhouse::openapi::Info::new("Roadmap", "0.1.0"))
        .build();

    assert_eq!(mounted::MOUNT.prefix(), "/inner");
    assert_eq!(
        built.oneshot_get("/inner/x/7").await.status(),
        axum::http::StatusCode::OK
    );
    assert_eq!(
        built.oneshot_get("/v3/inner/x/7").await.status(),
        axum::http::StatusCode::OK
    );
    assert!(document(&built)["paths"]["/v3/inner/x/{id}"]["get"].is_object());
}
