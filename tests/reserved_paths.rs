//! `try_build` is total: document routes and merged routers cannot unwind the build.

use roundhouse::prelude::*;
use roundhouse::{App, BuildError};

#[derive(Clone, Default)]
pub struct State;

mod document_path {
    use super::*;

    routes! {
           prefix = "/",
    get("/openapi.json", x, operation_id = "document_path_x") }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

#[test]
fn a_declared_route_cannot_claim_the_document_path() {
    let error = App::new(State)
        .include(&document_path::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Paths", "0.1.0"))
        .try_build()
        .err()
        .expect("the document route must be reserved");
    assert!(matches!(error, BuildError::RouteConflict { .. }));
    assert!(error.to_string().contains("OpenAPI document"));
}

mod docs_path {
    use super::*;

    routes! {
           prefix = "/",
    get("/docs", x, operation_id = "docs_path_x") }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

#[test]
fn a_declared_route_cannot_claim_the_docs_path() {
    let error = App::new(State)
        .include(&docs_path::MODULE)
        .docs()
        .try_build()
        .err()
        .expect("the docs route must be reserved");
    assert!(matches!(error, BuildError::RouteConflict { .. }));
    assert!(error.to_string().contains("documentation UI"));
}

#[test]
fn the_document_and_docs_routes_cannot_share_a_path() {
    let error = App::new(State)
        .include(&merged::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Paths", "0.1.0"))
        .docs_route("/openapi.json")
        .try_build()
        .err()
        .expect("the document and docs routes must differ");
    assert!(matches!(error, BuildError::RouteConflict { .. }));
}

mod merged {
    use super::*;

    routes! {
           prefix = "/merged",
    get("/x", x, operation_id = "merged_x") }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

#[test]
fn a_merged_router_collision_is_a_build_error() {
    let merged = axum::Router::new().route("/merged/x", axum::routing::get(|| async { "merged" }));
    let error = App::new(State)
        .include(&merged::MODULE)
        .merge(merged)
        .try_build()
        .err()
        .expect("a merged route conflict must be reported, not unwind");
    assert!(matches!(error, BuildError::RouteConflict { .. }));
    assert!(error.to_string().contains("App::merge"));
}
