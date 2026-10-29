//! Every path capture must become exactly one documented parameter.
//!
//! A scalar `Path<T>` is the idiomatic axum spelling and carries no parameter name, so the
//! document fills the name in from the template instead of omitting the parameter (which would
//! make the OpenAPI document invalid).

use roundhouse::prelude::*;
use roundhouse::{App, BuildError};

#[derive(Clone, Default)]
pub struct State;

fn scoped(module: &'static str) -> App<State> {
    App::new(State)
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Paths", "0.1.0"))
}

mod scalar {
    use super::*;

    mount!("/scalar");

    routes! { get("/users/{id}", show, operation_id = "scalar_show") }

    #[handler]
    pub async fn show(Path(id): Path<u64>) -> String {
        id.to_string()
    }
}

mod foreign {
    use super::*;

    mount!("/foreign");

    routes! { get("/users/{id}", show, operation_id = "foreign_show") }

    #[handler]
    pub async fn show(axum::extract::Path(id): axum::extract::Path<u64>) -> String {
        id.to_string()
    }
}

#[derive(Clone, Default, PathParams, serde::Deserialize)]
pub struct PostId {
    pub post_id: u64,
}

mod derived {
    use super::*;

    mount!("/derived");

    routes! { get("/posts/{post_id}", show, operation_id = "derived_show") }

    #[handler]
    pub async fn show(Path(PostId { post_id }): Path<PostId>) -> String {
        post_id.to_string()
    }
}

mod capture_in_mount {
    use super::*;

    mount!("/orgs/{org}");

    routes! { get("/members", list, operation_id = "mount_capture_list") }

    #[handler]
    pub async fn list() -> &'static str {
        "members"
    }
}

mod duplicate_capture {
    use super::*;

    mount!("/duplicate");

    routes! { get("/x/{id}/{id}", show, operation_id = "duplicate_capture_show") }

    #[handler]
    pub async fn show() -> &'static str {
        "x"
    }
}

mod default_ids {
    pub mod a {
        use roundhouse::prelude::*;

        mount!("/a");

        routes! { get("/{id}", show) }

        #[handler]
        pub async fn show() -> &'static str {
            "a"
        }
    }

    pub mod b {
        use roundhouse::prelude::*;

        mount!("/b");

        routes! { get("/{id}", show) }

        #[handler]
        pub async fn show() -> &'static str {
            "b"
        }
    }
}

#[test]
fn default_operation_ids_use_module_and_handler_not_the_url() {
    let app = scoped(concat!(module_path!(), "::default_ids"))
        .try_build()
        .expect("distinct resolved paths build");
    let ids: Vec<&str> = app
        .routes()
        .iter()
        .map(|route| route.operation_id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "get_paths_default_ids_a_show",
            "get_paths_default_ids_b_show"
        ]
    );
}

fn path_parameters(module: &'static str, path: &str) -> Vec<serde_json::Value> {
    let app = scoped(module).try_build().expect("valid document");
    let document = serde_json::to_value(app.document()).unwrap();
    document["paths"][path]["get"]["parameters"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn a_scalar_path_parameter_is_documented_from_the_template() {
    let parameters = path_parameters(concat!(module_path!(), "::scalar"), "/scalar/users/{id}");
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0]["name"], "id");
    assert_eq!(parameters[0]["in"], "path");
    assert_eq!(parameters[0]["required"], true);
    assert_eq!(parameters[0]["schema"]["type"], "integer");
    assert_eq!(parameters[0]["schema"]["format"], "u-int64");
}

#[test]
fn a_foreign_path_extractor_still_yields_a_valid_parameter() {
    let parameters = path_parameters(concat!(module_path!(), "::foreign"), "/foreign/users/{id}");
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0]["name"], "id");
    assert_eq!(parameters[0]["required"], true);
}

#[test]
fn a_derived_path_struct_documents_its_fields() {
    let parameters = path_parameters(
        concat!(module_path!(), "::derived"),
        "/derived/posts/{post_id}",
    );
    assert_eq!(parameters.len(), 1);
    assert_eq!(parameters[0]["name"], "post_id");
    assert_eq!(parameters[0]["schema"]["type"], "integer");
}

#[test]
fn a_capture_in_a_mount_prefix_is_supported() {
    let app = scoped(concat!(module_path!(), "::capture_in_mount"))
        .try_build()
        .expect("mount captures are supported");
    let route = &app.routes()[0];
    assert_eq!(route.path, "/orgs/{org}/members");
    assert_eq!(route.operation_id, "mount_capture_list");
}

#[test]
fn a_duplicate_capture_is_a_build_error() {
    let error = scoped(concat!(module_path!(), "::duplicate_capture"))
        .try_build()
        .err()
        .expect("duplicate captures are invalid");
    assert!(matches!(error, BuildError::InvalidTemplate { .. }));
}

mod asserted_path {
    use super::*;

    mount!("/asserted");

    routes! {
        get("/{id}", show, path = "/asserted/{id}"),
    }

    #[handler]
    pub async fn show() -> &'static str {
        "shown"
    }
}

mod wrong_path_assertion {
    use super::*;

    mount!("/moved");

    routes! {
        get("/x", show, path = "/old/x"),
    }

    #[handler]
    pub async fn show() -> &'static str {
        "shown"
    }
}

#[test]
fn a_path_assertion_accepts_the_resolved_path() {
    let app = scoped(concat!(module_path!(), "::asserted_path"))
        .try_build()
        .expect("the asserted path is correct");
    assert_eq!(app.routes()[0].path, "/asserted/{id}");
}

#[test]
fn a_path_assertion_names_both_sides_when_a_module_move_changes_the_url() {
    let error = scoped(concat!(module_path!(), "::wrong_path_assertion"))
        .try_build()
        .err()
        .expect("the assertion no longer matches");
    assert!(matches!(
        error,
        BuildError::PathAssertion { ref expected, ref actual, .. }
            if expected == "/old/x" && actual == "/moved/x"
    ));
    let message = error.to_string();
    assert!(message.contains("/old/x"));
    assert!(message.contains("/moved/x"));
}
