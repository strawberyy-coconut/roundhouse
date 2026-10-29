//! Captures in `mount!` prefixes, assembled with `merge`.
//!
//! The module tree is a grouping construct: every route is registered at its full resolved path.
//! Layers and handlers therefore see the full request URI, and `MatchedPath` is exactly the
//! document path.

use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use roundhouse::prelude::*;
use roundhouse::{App, BuildError};
use serde::Deserialize;
#[cfg(feature = "schema-engine")]
use serde::Serialize;

#[derive(Clone, Default)]
pub struct State;

fn scoped(module: &'static str) -> App<State> {
    App::new(State)
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Mount captures", "0.1.0"))
}

async fn full_uri(request: axum::extract::Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-uri",
        HeaderValue::try_from(path).expect("a request path is a valid header value"),
    );
    response
}

mod dynamic_parent {
    use super::*;

    mount!("/{project_id}/issues", tags = ["issues"]);

    routes! {
        get("/", list, operation_id = "issues_list"),
        get("/{issue_id}", show, operation_id = "issues_show"),
        get("/{issue_id}/matched", matched, operation_id = "issues_matched"),
    }

    #[derive(Clone, Default, Deserialize, PathParams)]
    pub struct ProjectPath {
        pub project_id: u64,
    }

    #[derive(Clone, Default, Deserialize, PathParams)]
    pub struct IssuePath {
        pub project_id: u64,
        pub issue_id: u64,
    }

    #[handler]
    pub async fn list(Path(ProjectPath { project_id }): Path<ProjectPath>) -> String {
        format!("project {project_id}")
    }

    #[handler]
    pub async fn show(
        Path(IssuePath {
            project_id,
            issue_id,
        }): Path<IssuePath>,
    ) -> String {
        format!("{project_id}/{issue_id}")
    }

    #[handler]
    pub async fn matched(path: axum::extract::MatchedPath) -> String {
        path.as_str().to_string()
    }
}

mod unconsumed {
    use super::*;

    mount!("/{org}/members");

    routes! { get("/", list, operation_id = "unconsumed_list") }

    #[handler]
    pub async fn list() -> &'static str {
        "members"
    }
}

mod unknown_parameter {
    use super::*;

    mount!("/{project_id}/issues");

    routes! { get("/{issue_id}", show, operation_id = "unknown_parameter_show") }

    #[derive(Clone, Default, Deserialize, PathParams)]
    pub struct Extra {
        pub issue_id: u64,
        pub bogus: u64,
    }

    #[handler]
    pub async fn show(Path(Extra { issue_id, .. }): Path<Extra>) -> String {
        issue_id.to_string()
    }
}

mod duplicate_mount_leaf {
    use super::*;

    mount!("/{id}");

    routes! { get("/{id}", show, operation_id = "duplicate_mount_leaf_show") }

    #[handler]
    pub async fn show() -> &'static str {
        "x"
    }
}

mod catch_all_mount {
    use super::*;

    mount!("/{*rest}");

    routes! { get("/x", broken, operation_id = "catch_all_mount_broken") }

    #[handler]
    pub async fn broken() -> &'static str {
        "x"
    }
}

mod old_syntax_mount {
    use super::*;

    mount!("/:id");

    routes! { get("/x", broken, operation_id = "old_syntax_mount_broken") }

    #[handler]
    pub async fn broken() -> &'static str {
        "x"
    }
}

mod malformed_mount {
    use super::*;

    mount!("/{id");

    routes! { get("/x", broken, operation_id = "malformed_mount_broken") }

    #[handler]
    pub async fn broken() -> &'static str {
        "x"
    }
}

mod param_suffix_mount {
    use super::*;

    mount!("/{bar}-foo");

    routes! { get("/x", broken, operation_id = "param_suffix_mount_broken") }

    #[handler]
    pub async fn broken() -> &'static str {
        "x"
    }
}

mod malformed_route {
    use super::*;

    mount!("/malformed");

    routes! { get("/{id", broken, operation_id = "malformed_route_broken") }

    #[handler]
    pub async fn broken() -> &'static str {
        "x"
    }
}

mod empty_capture_route {
    use super::*;

    mount!("/empty");

    routes! { get("/{}", broken, operation_id = "empty_capture_route_broken") }

    #[handler]
    pub async fn broken() -> &'static str {
        "x"
    }
}

mod param_suffix_route {
    use super::*;

    mount!("/suffix");

    routes! { get("/{bar}-foo", broken, operation_id = "param_suffix_route_broken") }

    #[handler]
    pub async fn broken() -> &'static str {
        "x"
    }
}

mod catch_all_leaf {
    use super::*;

    mount!("/{project_id}/files");

    routes! { get("/{*rest}", get_file, operation_id = "catch_all_leaf_get") }

    #[handler]
    pub async fn get_file(Path(rest): Path<std::collections::HashMap<String, String>>) -> String {
        rest.get("rest").cloned().unwrap_or_default()
    }
}

mod layered_capture {
    use super::*;

    mount!(
        "/{project_id}/issues",
        layer = middleware::from_fn(full_uri)
    );

    routes! { get("/{issue_id}", show, operation_id = "layered_capture_show") }

    #[handler]
    pub async fn show() -> &'static str {
        "ok"
    }
}

mod route_layered_parent {
    use super::*;

    mount!(
        "/rl/{project_id}",
        layer = middleware::from_fn(full_uri),
        layer_mode = route,
    );

    pub mod child {
        use roundhouse::prelude::*;

        routes! { get("/{issue_id}", show, operation_id = "route_layered_show") }

        #[handler]
        pub async fn show() -> &'static str {
            "ok"
        }
    }
}

mod shared {
    pub mod get {
        use roundhouse::prelude::*;

        mount!("/shared/{id}");

        routes! { get("/", get_thing, operation_id = "shared_get") }

        #[handler]
        pub async fn get_thing() -> &'static str {
            "get"
        }
    }

    pub mod post {
        use roundhouse::prelude::*;

        mount!("/shared/{id}");

        routes! { post("/", post_thing, operation_id = "shared_post") }

        #[handler]
        pub async fn post_thing() -> &'static str {
            "post"
        }
    }
}

mod conflicts {
    pub mod param {
        use roundhouse::prelude::*;

        mount!("/conflict");

        routes! { get("/{a}", a, operation_id = "conflict_param") }

        #[handler]
        pub async fn a() -> &'static str {
            "a"
        }
    }

    pub mod catch_all {
        use roundhouse::prelude::*;

        mount!("/conflict");

        routes! { get("/{*rest}", rest, operation_id = "conflict_catch_all") }

        #[handler]
        pub async fn rest() -> &'static str {
            "rest"
        }
    }
}

mod scoped_below {
    use super::*;

    mount!("/api/v1");

    pub mod issues {
        use roundhouse::prelude::*;

        routes! { get("/", list, operation_id = "scoped_below_list") }

        #[handler]
        pub async fn list() -> &'static str {
            "issues"
        }
    }
}

#[cfg(feature = "schema-engine")]
mod validated {
    use super::*;

    mount!("/{project_id}/reports");

    routes! {
        get("/{report_id}", show, operation_id = "validated_show"),
        get("/{report_id}/lying", lying, operation_id = "validated_lying"),
    }

    #[derive(Clone, Default, Deserialize, PathParams)]
    pub struct Keys {
        pub project_id: u64,
        pub report_id: u64,
    }

    #[derive(Serialize, ApiSchema)]
    pub struct Report {
        pub project_id: u64,
        pub report_id: u64,
    }

    #[handler]
    pub async fn show(
        Path(Keys {
            project_id,
            report_id,
        }): Path<Keys>,
    ) -> Json<Report> {
        Json(Report {
            project_id,
            report_id,
        })
    }

    #[derive(Serialize, ApiSchema)]
    pub struct Lying {
        #[schema(min_length = 3)]
        pub name: String,
    }

    #[handler]
    pub async fn lying() -> Json<Lying> {
        Json(Lying {
            name: "x".to_string(),
        })
    }
}

fn path_parameters(app: &roundhouse::Built, path: &str, method: &str) -> Vec<serde_json::Value> {
    let document = serde_json::to_value(app.document().expect("document enabled")).unwrap();
    document["paths"][path][method]["parameters"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn a_mount_capture_reaches_a_handler_on_the_leaf_index() {
    let app = scoped(concat!(module_path!(), "::dynamic_parent")).build();
    let response = app.oneshot_get("/42/issues").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"project 42");
}

#[tokio::test]
async fn mount_and_leaf_captures_deserialize_together() {
    let app = scoped(concat!(module_path!(), "::dynamic_parent")).build();
    let response = app.oneshot_get("/42/issues/7").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"42/7");
}

#[tokio::test]
async fn matched_path_is_the_full_document_path() {
    let app = scoped(concat!(module_path!(), "::dynamic_parent")).build();
    let response = app.oneshot_get("/42/issues/7/matched").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"/{project_id}/issues/{issue_id}/matched");
}

#[test]
fn both_captures_are_documented_with_the_path_struct_schema() {
    let app = scoped(concat!(module_path!(), "::dynamic_parent")).build();

    let list = path_parameters(&app, "/{project_id}/issues", "get");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "project_id");
    assert_eq!(list[0]["schema"]["type"], "integer");

    let show = path_parameters(&app, "/{project_id}/issues/{issue_id}", "get");
    let names: Vec<&str> = show
        .iter()
        .map(|parameter| parameter["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["project_id", "issue_id"]);
    assert!(show.iter().all(|parameter| parameter["in"] == "path"));
    assert!(
        show.iter()
            .all(|parameter| parameter["schema"]["type"] == "integer")
    );
}

#[test]
fn an_unconsumed_mount_capture_is_documented_as_a_string_with_a_diagnostic() {
    let app = scoped(concat!(module_path!(), "::unconsumed")).build();
    let parameters = path_parameters(&app, "/{org}/members", "get");
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0]["name"], "org");
    assert_eq!(parameters[0]["schema"]["type"], "string");

    let diagnostic = app
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.message.contains("documented as a string"))
        .expect("the synthesized capture is called out");
    assert!(diagnostic.message.contains("org"));
}

#[test]
fn strict_escalates_the_synthesized_capture_diagnostic() {
    let error = scoped(concat!(module_path!(), "::unconsumed"))
        .strict()
        .try_build()
        .err()
        .expect("strict promotes the diagnostic");
    assert!(matches!(error, BuildError::Diagnostic { .. }));
}

#[test]
fn a_path_name_outside_the_full_template_is_a_build_error() {
    let error = scoped(concat!(module_path!(), "::unknown_parameter"))
        .try_build()
        .err()
        .expect("bogus is not a capture");
    assert!(matches!(
        error,
        BuildError::UnknownPathParameter { name, .. } if name == "bogus"
    ));
}

#[test]
fn a_duplicate_capture_between_mount_and_leaf_is_rejected() {
    let error = scoped(concat!(module_path!(), "::duplicate_mount_leaf"))
        .try_build()
        .err()
        .expect("the full template repeats `id`");
    assert!(matches!(error, BuildError::InvalidTemplate { .. }));
}

#[test]
fn a_catch_all_in_a_mount_is_rejected() {
    let error = scoped(concat!(module_path!(), "::catch_all_mount"))
        .try_build()
        .err()
        .expect("a mount cannot end in a catch-all");
    assert!(matches!(error, BuildError::InvalidMountPrefix { .. }));
}

#[test]
fn old_axum_syntax_in_a_mount_is_rejected() {
    let error = scoped(concat!(module_path!(), "::old_syntax_mount"))
        .try_build()
        .err()
        .expect("`:id` is axum 0.7 syntax");
    assert!(matches!(error, BuildError::InvalidMountPrefix { .. }));
}

#[test]
fn malformed_mount_prefixes_are_build_errors_not_panics() {
    for module in ["malformed_mount", "param_suffix_mount"] {
        let name = match module {
            "malformed_mount" => concat!(module_path!(), "::malformed_mount"),
            _ => concat!(module_path!(), "::param_suffix_mount"),
        };
        let error = scoped(name)
            .try_build()
            .err()
            .unwrap_or_else(|| panic!("`{module}` should be rejected"));
        assert!(matches!(error, BuildError::InvalidMountPrefix { .. }));
    }
}

#[test]
fn malformed_leaf_templates_are_build_errors_not_panics() {
    for module in [
        "malformed_route",
        "empty_capture_route",
        "param_suffix_route",
    ] {
        let name = match module {
            "malformed_route" => concat!(module_path!(), "::malformed_route"),
            "empty_capture_route" => concat!(module_path!(), "::empty_capture_route"),
            _ => concat!(module_path!(), "::param_suffix_route"),
        };
        let error = scoped(name)
            .try_build()
            .err()
            .unwrap_or_else(|| panic!("`{module}` should be rejected"));
        assert!(matches!(error, BuildError::InvalidTemplate { .. }));
    }
}

#[tokio::test]
async fn a_leaf_catch_all_works_under_a_dynamic_mount() {
    let app = scoped(concat!(module_path!(), "::catch_all_leaf")).build();
    let response = app.oneshot_get("/9/files/a/b.txt").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"a/b.txt");
}

#[tokio::test]
async fn a_layer_on_a_dynamic_mount_sees_the_full_uri() {
    let app = scoped(concat!(module_path!(), "::layered_capture")).build();
    let response = app.oneshot_get("/42/issues/7").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-uri"], "/42/issues/7");

    let response = app.oneshot_get("/42/issues").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get("x-uri").is_none());
}

#[tokio::test]
async fn a_parent_route_layer_wraps_child_routes_but_not_the_outer_404() {
    let app = scoped(concat!(module_path!(), "::route_layered_parent")).build();

    let response = app.oneshot_get("/rl/42/7").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-uri"], "/rl/42/7");

    let response = app.oneshot_get("/rl/42/7/extra").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get("x-uri").is_none());
}

#[tokio::test]
async fn a_method_mismatch_under_a_dynamic_mount_is_405_with_allow() {
    let app = scoped(concat!(module_path!(), "::dynamic_parent")).build();
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(http::Method::POST)
                .uri("/42/issues/7")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let allow = response
        .headers()
        .get(header::ALLOW)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(allow.contains("GET"), "Allow was `{allow}`");
}

#[tokio::test]
async fn the_same_full_path_merges_methods_from_different_modules() {
    let app = scoped(concat!(module_path!(), "::shared")).build();

    let response = app.oneshot_get("/shared/9").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"get");

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method(http::Method::POST)
                .uri("/shared/9")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"post");
}

#[test]
fn overlapping_shapes_are_a_route_conflict_not_a_panic() {
    let error = scoped(concat!(module_path!(), "::conflicts"))
        .try_build()
        .err()
        .expect("`{a}` and `{*rest}` share a matchit position");
    assert!(matches!(error, BuildError::RouteConflict { .. }));
}

#[test]
fn the_route_listing_uses_full_paths() {
    let listing = scoped(concat!(module_path!(), "::dynamic_parent")).route_listing();
    assert!(listing.contains("/{project_id}/issues"));
    assert!(listing.contains("/{project_id}/issues/{issue_id}"));
    assert!(listing.contains("mount_captures.rs"));
}

#[test]
fn a_scope_below_a_mount_drops_the_ancestor_prefix() {
    // `scope` selects declarations, not a URL subtree. A scope inside a mounted module therefore
    // does not inherit the mount; the route is served at its leaf path. Documented in the README.
    let app = scoped(concat!(module_path!(), "::scoped_below::issues")).build();
    assert_eq!(app.routes().len(), 1);
    assert_eq!(app.routes()[0].path, "/");
}

#[cfg(feature = "schema-engine")]
#[tokio::test]
async fn validate_responses_matches_the_document_under_a_dynamic_mount() {
    let app = scoped(concat!(module_path!(), "::validated"))
        .validate_responses()
        .build();
    let response = app.oneshot_get("/9/reports/3").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["project_id"], 9);
    assert_eq!(value["report_id"], 3);
}

#[cfg(feature = "schema-engine")]
#[tokio::test]
async fn response_validation_actually_runs_under_a_dynamic_mount() {
    // If `MatchedPath` were the leaf path, the validator would have no entry, skip the body, and
    // this deliberately invalid response would leak out as a 200.
    let app = scoped(concat!(module_path!(), "::validated"))
        .validate_responses()
        .build();
    let response = app.oneshot_get("/9/reports/3/lying").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("response validation failed"), "body: {text}");
}
