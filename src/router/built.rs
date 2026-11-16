#[cfg(feature = "contract")]
use std::sync::{Arc, OnceLock};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use tower::ServiceExt;

use crate::auth::AuthGuard;
#[cfg(feature = "contract")]
use crate::doc::{self, OperationDoc};
#[cfg(feature = "contract")]
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
    /// `anonymous` cleared inherited guards for this operation. The document emits
    /// `security: []` so the operation opts out of any document-level requirement.
    pub auth_cleared: bool,
    pub hidden: bool,
    pub layer_docs: Vec<crate::doc::layer::LayerDoc>,
}

impl ResolvedRoute {
    #[cfg(feature = "contract")]
    pub fn docs(&self) -> OperationDoc {
        let mut components = utoipa::openapi::schema::Components::new();
        let mut context = crate::doc::DocContext::new(&mut components);
        (self.record.docs)(&mut context)
    }
}

/// The assembled application.
#[derive(Clone)]
pub struct Built {
    pub(crate) router: Router<()>,
    pub(crate) routes: Vec<ResolvedRoute>,
    #[cfg(feature = "contract")]
    pub(crate) document: Arc<OnceLock<OpenApi>>,
    /// The verbatim `openapi.json` from the contract bundle, for a runtime-only build.
    #[cfg(feature = "contract-runtime")]
    #[allow(dead_code)]
    pub(crate) contract_openapi: Option<&'static str>,
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

    /// The generated document.
    ///
    /// In contract mode this parses the committed `openapi.json` on first use, so a runtime
    /// application does not pay for a view it never reads.
    #[cfg(feature = "contract")]
    pub fn document(&self) -> &OpenApi {
        self.document.get_or_init(|| {
            let text = self
                .contract_openapi
                .expect("contract mode has an embedded openapi.json");
            serde_json::from_str(text)
                .unwrap_or_else(|error| panic!("the embedded openapi.json is invalid: {error}"))
        })
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

    #[cfg(feature = "contract")]
    #[cfg(feature = "contract")]
    pub fn openapi_json(&self) -> String {
        self.contract_openapi.unwrap_or_default().to_string()
    }

    /// The committed `openapi.json`, verbatim.
    #[cfg(all(feature = "contract-runtime", not(feature = "contract")))]
    pub fn openapi_json(&self) -> String {
        self.contract_openapi.unwrap_or_default().to_string()
    }

    /// A 3.0.3-shaped JSON document.
    #[cfg(feature = "contract")]
    pub fn openapi_30(&self) -> serde_json::Value {
        doc::downgrade::to_openapi_30(self.document().as_utoipa())
    }

    #[cfg(feature = "contract")]
    pub fn openapi_30_json(&self) -> String {
        serde_json::to_string_pretty(&self.openapi_30()).expect("JSON serializes")
    }

    /// Send one request through the router.
    ///
    /// Test helper: prefer [`Self::router`] in production code.
    #[doc(hidden)]
    pub async fn oneshot(&self, request: Request<Body>) -> Response {
        self.router
            .clone()
            .oneshot(request)
            .await
            .expect("axum Router is infallible")
    }

    /// `GET` one URI, for tests.
    #[doc(hidden)]
    pub async fn oneshot_get(&self, uri: &str) -> Response {
        let request = Request::builder()
            .method(http::Method::GET)
            .uri(uri)
            .body(Body::empty())
            .expect("valid test request");
        self.oneshot(request).await
    }

    /// Send a JSON request, for tests.
    #[doc(hidden)]
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
