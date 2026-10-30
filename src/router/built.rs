use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use tower::ServiceExt;

use crate::auth::AuthGuard;
use crate::doc::{self, HandlerDocs};
use crate::openapi::OpenApi;
use crate::registry::{Method, RouteRecord};

use super::fragment::MergeRecord;

/// A resolved route: method + full path + everything the document needs.
#[derive(Clone)]
pub struct ResolvedRoute {
    pub method: Method,
    pub path: String,
    /// The explicit `operation_id` or one derived from method, path and module.
    pub operation_id: String,
    pub module_path: &'static str,
    pub record: &'static RouteRecord,
    pub tags: Vec<&'static str>,
    /// Guards inherited from modules plus this entry's own `security`; installed on the
    /// method router unless the handler already requires the same scheme.
    pub auth: Vec<AuthGuard>,
    pub hidden: bool,
    pub layer_docs: Vec<crate::doc::layer::LayerDoc>,
}

impl ResolvedRoute {
    pub fn docs(&self) -> HandlerDocs {
        let mut components = utoipa::openapi::schema::Components::new();
        (self.record.docs)(&mut components, self.record.validate)
    }
}

/// The assembled application.
#[derive(Clone)]
pub struct Built {
    pub(crate) router: Router<()>,
    pub(crate) routes: Vec<ResolvedRoute>,
    pub(crate) document: Arc<OpenApi>,
    pub(crate) merges: Vec<MergeRecord>,
    pub(crate) diagnostics: Vec<crate::build_error::Diagnostic>,
}

impl Built {
    /// The ready `axum::Router`.
    pub fn router(&self) -> Router<()> {
        self.router.clone()
    }

    /// The resolved route table, sorted by path then method.
    pub fn routes(&self) -> &[ResolvedRoute] {
        &self.routes
    }

    /// The generated document. It is always built; `App::openapi(..)` controls whether it is
    /// also served at a route.
    pub fn document(&self) -> &OpenApi {
        &self.document
    }

    /// Hand-written routers merged into this app, with the `App::merge` call site.
    ///
    /// Every entry means the served router has routes the document does not describe.
    pub fn merges(&self) -> &[MergeRecord] {
        &self.merges
    }

    /// Non-fatal observations collected while resolving and documenting the app.
    pub fn diagnostics(&self) -> &[crate::build_error::Diagnostic] {
        &self.diagnostics
    }

    pub fn openapi_json(&self) -> String {
        self.document
            .to_pretty_json()
            .expect("OpenAPI document serializes")
    }

    /// A 3.0.3-shaped JSON document.
    pub fn openapi_30(&self) -> serde_json::Value {
        doc::downgrade::to_openapi_30(self.document.as_utoipa())
    }

    pub fn openapi_30_json(&self) -> String {
        serde_json::to_string_pretty(&self.openapi_30()).expect("JSON serializes")
    }

    /// Send one request through the router.
    pub async fn oneshot(&self, request: Request<Body>) -> Response {
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("axum Router is infallible")
    }

    pub async fn oneshot_get(&self, uri: &str) -> Response {
        let request = Request::builder()
            .method(http::Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("valid test request");
        self.oneshot(request).await
    }

    pub async fn oneshot_json(&self, method: http::Method, uri: &str, body: &str) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .expect("valid test request");
        self.oneshot(request).await
    }
}
