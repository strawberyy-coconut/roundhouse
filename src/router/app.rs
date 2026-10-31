use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::handler::Handler;
use axum::http::Request;
use axum::response::IntoResponse;
use axum::routing::Route;
use tower::{Layer, Service};

use crate::openapi::{Info, OpenApi};
use crate::registry::Module;

use super::fragment::{DocumentRejections, Fragment, PendingMerge};

pub(crate) type RouterLayer = Box<dyn Fn(Router<()>) -> Router<()> + Send + Sync>;
pub(crate) type DocumentHook = Box<dyn Fn(&mut OpenApi) + Send + Sync>;

pub(crate) type ValidationDefaults = (
    axum::http::StatusCode,
    crate::extract::Validation,
    Option<usize>,
    Option<usize>,
    Option<Arc<roundhouse_schema::engine::SchemaEngine>>,
);

/// The application builder.
///
/// `App::router()` returns a real `axum::Router`; `App::build()` gives the same router plus the
/// resolved table and the cached OpenAPI document.
pub struct App<S> {
    pub(crate) state: S,
    pub(crate) roots: Vec<&'static Module>,
    pub(crate) global_layers: Vec<RouterLayer>,
    pub(crate) global_layer_docs: Vec<fn() -> crate::doc::layer::LayerDoc>,
    pub(crate) fallback: Option<RouterLayer>,
    pub(crate) merges: Vec<PendingMerge>,
    pub(crate) info: Option<Box<dyn Fn() -> Info + Send + Sync>>,
    pub(crate) servers: Vec<utoipa::openapi::Server>,
    pub(crate) document_hooks: Vec<DocumentHook>,
    pub(crate) openapi_route: Option<&'static str>,
    pub(crate) docs_route: Option<&'static str>,
    pub(crate) document_rejections: DocumentRejections,
    pub(crate) validation_status: axum::http::StatusCode,
    pub(crate) validate_responses: bool,
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
            global_layer_docs: Vec::new(),
            fallback: None,
            merges: Vec::new(),
            info: None,
            servers: Vec::new(),
            document_hooks: Vec::new(),
            openapi_route: None,
            docs_route: None,
            document_rejections: DocumentRejections::Auto,
            validation_status: axum::http::StatusCode::BAD_REQUEST,
            validate_responses: false,
            max_body: None,
            #[cfg(feature = "multipart")]
            max_parts: None,
            strict: false,
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

    /// Apply a tower layer globally and document what it guarantees (§6.1).
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
    /// `x-roundhouse-merged` and a warning is emitted; `App::strict()` turns it into an error.
    #[track_caller]
    pub fn merge(mut self, router: Router<()>) -> Self {
        let location = std::panic::Location::caller();
        self.merges.push(PendingMerge {
            router,
            source: crate::build_error::Source::new("App::merge", location.file(), location.line()),
        });
        self
    }

    /// Enable `/openapi.json` and set the document info.
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
    pub fn server(mut self, server: impl Into<utoipa::openapi::Server>) -> Self {
        self.servers.push(server.into());
        self
    }

    /// Add several `servers` entries to the document.
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
    pub fn document<F>(mut self, hook: F) -> Self
    where
        F: Fn(&mut OpenApi) + Send + Sync + 'static,
    {
        self.document_hooks.push(Box::new(hook));
        self
    }

    /// Choose a different path for the generated document.
    pub fn openapi_route(mut self, path: &'static str) -> Self {
        self.openapi_route = Some(path);
        self
    }

    /// Serve a Swagger UI page at `/docs` over the generated document.
    ///
    /// This implies the document route: a UI without a document is a broken page. The UI assets
    /// are pinned (and SRI-checked) but loaded from unpkg, so the page needs browser network
    /// access; serve your own UI at the document route for an air-gapped deployment.
    pub fn docs(mut self) -> Self {
        self.docs_route = Some("/docs");
        if self.openapi_route.is_none() {
            self.openapi_route = Some("/openapi.json");
        }
        self
    }

    /// Serve the docs UI at a custom path. Implies the document route.
    pub fn docs_route(mut self, path: &'static str) -> Self {
        self.docs_route = Some(path);
        if self.openapi_route.is_none() {
            self.openapi_route = Some("/openapi.json");
        }
        self
    }

    /// Turn the loud-derivation warnings into build errors.
    pub fn strict(mut self) -> Self {
        self.strict = true;
        self
    }

    /// Add or suppress extractor rejection responses (`Auto` by default).
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
    /// This is the default for every body extractor, including multipart when that feature is
    /// enabled, where a field's `max_bytes` can only narrow it.
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

    /// Validate JSON responses against the generated document at runtime.
    ///
    /// Intended for tests and debug builds: a mismatch becomes a `500 Problem` naming the JSON
    /// Pointer instead of shipping a document the server does not honor.
    pub fn validate_responses(mut self) -> Self {
        self.validate_responses = true;
        self
    }
}
