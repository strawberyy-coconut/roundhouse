use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::handler::Handler;
use axum::http::Request;
use axum::response::IntoResponse;
use axum::routing::Route;
use tower::{Layer, Service};

#[cfg(feature = "build")]
use crate::openapi::{Info, OpenApi};
use crate::registry::Module;

#[cfg(feature = "build")]
use super::fragment::DocumentRejections;
use super::fragment::{Fragment, PendingMerge};

pub(crate) type RouterLayer = Box<dyn Fn(Router<()>) -> Router<()> + Send + Sync>;
#[cfg(feature = "build")]
pub(crate) type DocumentHook = Box<dyn Fn(&mut OpenApi) + Send + Sync>;

/// Request-scoped validation configuration installed once for the whole app.
#[derive(Clone)]
pub(crate) struct ValidationDefaults {
    pub(crate) status: axum::http::StatusCode,
    pub(crate) mode: crate::extract::Validation,
    pub(crate) max_body: Option<usize>,
    pub(crate) max_parts: Option<usize>,
    pub(crate) engine: Option<Arc<roundhouse_schema::engine::SchemaEngine>>,
}

/// The application builder.
///
/// `App::into_router()` returns the ready `axum::Router<()>`; `App::build()` gives the same router
/// plus the resolved table and the cached OpenAPI document.
pub struct App<S> {
    pub(crate) state: S,
    pub(crate) roots: Vec<&'static Module>,
    pub(crate) global_layers: Vec<RouterLayer>,
    #[cfg(feature = "build")]
    pub(crate) global_layer_docs: Vec<fn() -> crate::doc::layer::LayerDoc>,
    pub(crate) fallback: Option<RouterLayer>,
    pub(crate) merges: Vec<PendingMerge>,
    #[cfg(feature = "build")]
    pub(crate) info: Option<Box<dyn Fn() -> Info + Send + Sync>>,
    #[cfg(feature = "build")]
    pub(crate) servers: Vec<utoipa::openapi::Server>,
    #[cfg(feature = "build")]
    pub(crate) document_hooks: Vec<DocumentHook>,
    #[cfg(feature = "contract")]
    pub(crate) openapi_route: Option<&'static str>,
    #[cfg(feature = "contract")]
    pub(crate) docs_route: Option<&'static str>,
    /// Load the docs UI from unpkg instead of the vendored assets.
    #[cfg(feature = "contract")]
    pub(crate) docs_cdn: bool,
    #[cfg(feature = "swagger-ui")]
    #[cfg(feature = "contract")]
    pub(crate) swagger_route: Option<&'static str>,
    #[cfg(feature = "build")]
    pub(crate) document_rejections: DocumentRejections,
    /// When set, the OpenAPI route serves this build-time artifact. The runtime never assembles
    /// a document.
    #[cfg(feature = "contract")]
    pub(crate) contract_bundle: Option<crate::ContractBundle>,
    pub(crate) validation_status: axum::http::StatusCode,
    /// Verify actual responses against the committed contract (tests, `cargo roundhouse verify`).
    #[cfg(feature = "contract")]
    pub(crate) verify_responses: bool,
    pub(crate) max_body: Option<usize>,
    #[cfg(feature = "multipart")]
    pub(crate) max_parts: Option<usize>,
    pub(crate) strict: bool,
    pub(crate) validation: crate::extract::Validation,
}

impl<S> App<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new(state: S) -> Self {
        Self {
            state,
            roots: Vec::new(),
            global_layers: Vec::new(),
            #[cfg(feature = "build")]
            global_layer_docs: Vec::new(),
            fallback: None,
            merges: Vec::new(),
            #[cfg(feature = "build")]
            info: None,
            #[cfg(feature = "build")]
            servers: Vec::new(),
            #[cfg(feature = "build")]
            document_hooks: Vec::new(),
            #[cfg(feature = "contract")]
            openapi_route: None,
            #[cfg(feature = "contract")]
            docs_route: None,
            #[cfg(feature = "contract")]
            docs_cdn: false,
            #[cfg(feature = "contract")]
            #[cfg(feature = "swagger-ui")]
            swagger_route: None,
            #[cfg(feature = "build")]
            document_rejections: DocumentRejections::Auto,
            #[cfg(feature = "contract")]
            contract_bundle: None,
            validation_status: axum::http::StatusCode::BAD_REQUEST,
            #[cfg(feature = "contract")]
            verify_responses: false,
            max_body: Some(crate::extract::DEFAULT_MAX_BODY),
            #[cfg(feature = "multipart")]
            max_parts: None,
            strict: true,
            validation: crate::extract::Validation::default(),
        }
    }

    /// Root the app at one module tree emitted by `routes!`.
    ///
    /// Compose several modules with [`App::fragment`] or by spreading them in a `routes!` table.
    /// The module tree is the complete route set: nothing is discovered from the linker.
    pub fn include(mut self, module: &'static Module) -> Self {
        self.roots.push(module);
        self
    }

    /// Root the app at an explicit [`Fragment`].
    ///
    /// This is what `app!(state)` and `app!(state, module)` call; the builder form is useful for
    /// tests, libraries and applications that choose a route subset at runtime.
    pub fn fragment(mut self, fragment: Fragment) -> Self {
        self.roots.extend(fragment.roots().iter().copied());
        self
    }

    /// Apply a tower layer globally, outermost.
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request<Body>> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request<Body>>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request<Body>>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request<Body>>>::Future: Send + 'static,
    {
        self.global_layers
            .push(Box::new(move |router| router.layer(layer.clone())));
        self
    }

    /// Apply a tower layer globally and document what it guarantees.
    #[cfg(feature = "build")]
    pub fn layer_documented<L>(self, layer: L, doc: fn() -> crate::doc::layer::LayerDoc) -> Self
    where
        L: Layer<Route> + Clone + Send + Sync + 'static,
        L::Service: Service<Request<Body>> + Clone + Send + Sync + 'static,
        <L::Service as Service<Request<Body>>>::Response: IntoResponse + 'static,
        <L::Service as Service<Request<Body>>>::Error: Into<Infallible> + 'static,
        <L::Service as Service<Request<Body>>>::Future: Send + 'static,
    {
        let mut app = self.layer(layer);
        app.global_layer_docs.push(doc);
        app
    }

    /// Set the fallback for unmatched requests. The default is a JSON `404 Problem`.
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        H: Handler<T, ()> + Clone + Send + Sync + 'static,
        T: 'static,
    {
        self.fallback = Some(Box::new(move |router| router.fallback(handler.clone())));
        self
    }

    /// Merge a hand-written router. Escape hatch in the other direction.
    ///
    /// Merged routes are served but cannot be discovered for the document. The document is marked
    /// `x-roundhouse-merged`, and the merge is a build error by default. Use
    /// [`App::merge_undocumented`] to acknowledge the missing documentation explicitly.
    #[track_caller]
    pub fn merge(self, router: Router<()>) -> Self {
        let location = std::panic::Location::caller();
        self.push_merge(router, location, None)
    }

    /// Merge a hand-written router and acknowledge that its routes are undocumented.
    ///
    /// Prefer [`App::merge`] when the router can be replaced by `routes!` declarations. This
    /// method exists for routers that cannot be expressed as roundhouse routes; it records the
    /// author's reason in [`crate::Built::merges`] and keeps the document's `x-roundhouse-merged`
    /// marker.
    #[track_caller]
    pub fn merge_undocumented(self, router: Router<()>, reason: &'static str) -> Self {
        let location = std::panic::Location::caller();
        self.push_merge(router, location, Some(reason))
    }

    #[track_caller]
    fn push_merge(
        mut self,
        router: Router<()>,
        location: &'static std::panic::Location<'static>,
        reason: Option<&'static str>,
    ) -> Self {
        self.merges.push(PendingMerge {
            router,
            source: crate::build_error::Source::new("App::merge", location.file(), location.line()),
            reason,
        });
        self
    }

    /// Serve a build-time contract artifact at the document route.
    ///
    /// `bundle` is normally produced by [`include_contract!`](crate::include_contract). The
    /// runtime validates the artifact's kind and layout version, compiles its request plans when
    /// schema validation is enabled, and serves `openapi` verbatim.
    #[cfg(feature = "contract")]
    pub fn contract(mut self, bundle: crate::ContractBundle) -> Self {
        self.contract_bundle = Some(bundle);
        self
    }

    /// Enable `/openapi.json` and set the document info.
    #[cfg(feature = "build")]
    pub fn openapi<F, I>(mut self, info: F) -> Self
    where
        F: Fn() -> I + Send + Sync + 'static,
        I: Into<Info>,
    {
        self.info = Some(Box::new(move || info().into()));
        self.openapi_route = Some("/openapi.json");
        self
    }

    /// Add one `servers` entry to the document.
    ///
    /// Servers are deployment facts, so they come from the app rather than from a route
    /// declaration.
    #[cfg(feature = "build")]
    pub fn server(mut self, server: impl Into<utoipa::openapi::Server>) -> Self {
        self.servers.push(server.into());
        self
    }

    /// Add several `servers` entries to the document.
    #[cfg(feature = "build")]
    pub fn servers<I, T>(mut self, servers: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<utoipa::openapi::Server>,
    {
        self.servers.extend(servers.into_iter().map(Into::into));
        self
    }

    /// Run a hook over the finished document before it is served or returned.
    ///
    /// This is the escape hatch for document metadata the declarations cannot derive, such as
    /// `tags` with descriptions. Hooks run in registration order after route assembly.
    ///
    /// Hooks are metadata-only: the schema engine is compiled from the route declarations, so a
    /// hook must not change request or response validation schemas.
    #[cfg(feature = "build")]
    pub fn document<F>(mut self, hook: F) -> Self
    where
        F: Fn(&mut OpenApi) + Send + Sync + 'static,
    {
        self.document_hooks.push(Box::new(hook));
        self
    }

    /// Choose a different path for the generated document.
    #[cfg(feature = "contract")]
    pub fn openapi_route(mut self, path: &'static str) -> Self {
        self.openapi_route = Some(path);
        self
    }

    /// Serve a Swagger UI page at `/docs` over the generated document.
    ///
    /// This implies the document route: a UI without a document is a broken page. With the
    /// `swagger-ui` feature (enabled by default), the CSS, JavaScript and OAuth assets are served by
    /// this app, so the page works in an air-gapped deployment. Use [`App::docs_cdn`] to load the
    /// pinned unpkg assets instead.
    #[cfg(feature = "contract")]
    pub fn docs(mut self) -> Self {
        self.docs_route = Some("/docs");
        if self.openapi_route.is_none() {
            self.openapi_route = Some("/openapi.json");
        }
        self
    }

    /// Serve the docs UI at a custom path. Implies the document route.
    #[cfg(feature = "contract")]
    pub fn docs_route(mut self, path: &'static str) -> Self {
        self.docs_route = Some(path);
        if self.openapi_route.is_none() {
            self.openapi_route = Some("/openapi.json");
        }
        self
    }

    /// Load the docs UI assets from unpkg instead of serving them locally.
    ///
    /// The unpkg assets are pinned and SRI-checked, but the browser needs network access. Without
    /// the `swagger-ui` feature this is the only available mode, so the call is a no-op there.
    #[cfg(feature = "contract")]
    pub fn docs_cdn(mut self) -> Self {
        self.docs_cdn = true;
        self
    }

    /// Serve a self-hosted Swagger UI at `/docs/swagger` over the generated document.
    ///
    /// Unlike [`App::docs`], the page loads its CSS, JavaScript and OAuth redirect assets from
    /// routes served by this app, so it works without browser network access. This implies the
    /// document route and can be used alongside [`App::docs`].
    #[cfg(feature = "swagger-ui")]
    #[cfg(feature = "contract")]
    pub fn swagger(mut self) -> Self {
        self.swagger_route = Some("/docs/swagger");
        if self.openapi_route.is_none() {
            self.openapi_route = Some("/openapi.json");
        }
        self
    }

    /// Serve the self-hosted Swagger UI at a custom path. Implies the document route.
    #[cfg(feature = "swagger-ui")]
    #[cfg(feature = "contract")]
    pub fn swagger_route(mut self, path: &'static str) -> Self {
        self.swagger_route = Some(path);
        if self.openapi_route.is_none() {
            self.openapi_route = Some("/openapi.json");
        }
        self
    }

    /// Keep strict builds. This is the default.
    ///
    /// Every route is expected to truthfully describe its body, responses and extractors. An
    /// unverified surface fails [`App::try_build`] unless the route acknowledges it with
    /// `unverified = "reason"` (or opts out explicitly with `validate_body = false` where that
    /// applies).
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    /// Downgrade build diagnostics to warnings.
    ///
    /// This is the explicit escape hatch for an application that cannot make every route honest
    /// yet. The warnings are still collected in [`crate::Built::diagnostics`] and printed by
    /// [`App::build`]. Prefer fixing the declarations or acknowledging them per route.
    pub fn permissive(mut self) -> Self {
        self.strict = false;
        self
    }

    /// Add or suppress extractor rejection responses (`Auto` by default).
    #[cfg(feature = "build")]
    pub fn document_rejections(mut self, mode: impl Into<DocumentRejections>) -> Self {
        self.document_rejections = mode.into();
        self
    }

    /// Status returned by constraint validation. `400` by default; `422` is a common alternative.
    pub fn validation_status(mut self, status: axum::http::StatusCode) -> Self {
        self.validation_status = status;
        self
    }

    /// Choose the enforcer: the emitted JSON Schema (default) or none.
    pub fn validation(mut self, mode: crate::extract::Validation) -> Self {
        self.validation = mode;
        self
    }

    /// Set the global request body limit. Body extractors answer `413` above it.
    ///
    /// The default is 2 MiB. This applies to every body extractor, including multipart when that
    /// feature is enabled, where a field's `max_bytes` can only narrow it.
    pub fn max_body(mut self, bytes: usize) -> Self {
        self.max_body = Some(bytes);
        self
    }

    /// Bound the number of parts in a multipart request. Many tiny parts are a DoS axis.
    #[cfg(feature = "multipart")]
    pub fn max_parts(mut self, parts: usize) -> Self {
        self.max_parts = Some(parts);
        self
    }

    /// Verify actual responses against the committed contract.
    ///
    /// Intended for tests and `cargo roundhouse verify`: a response whose status, content type or
    /// JSON body does not match `contract.json` becomes a `500 Problem`. The
    /// `ROUNDHOUSE_VERIFY` environment variable enables the same check without a code change.
    #[cfg(feature = "contract")]
    pub fn verify_responses(mut self) -> Self {
        self.verify_responses = true;
        self
    }
}
