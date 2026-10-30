//! The application builder, route resolution and router assembly.
//!
//! [`App`] starts from application state and an explicit module tree. In application code, prefer
//! the [`app!`](crate::app!) macro: `app!(state)` roots the app at the fragment declared in the
//! calling module, while `app!(state, module)` names another module's fragment. Builder methods
//! then configure layers, the document, validation and request limits. [`App::build`] walks the
//! tree into a [`Built`]:
//!
//! - [`Built::router`] — the ready `axum::Router`;
//! - [`Built::document`] and [`Built::openapi_json`] — the OpenAPI 3.1 document;
//! - [`Built::routes`] — the resolved table, with each declaration's source location.
//!
//! A single `routes!` table rejects its own duplicate routes, duplicate operation ids and missing
//! success responses during expansion, so `cargo check` catches those. Resolution of the composed
//! tree is still fallible at build time: collisions across modules, prefix joins, state
//! mismatches and the final document need the aggregate view. Use [`App::try_build`] in a test
//! or a check binary so CI can fail without starting a server.
//! [`App::router`] is the escape hatch that returns a plain `axum::Router<S>` for embedding in a
//! larger axum application.
//!
//! ```no_run
//! # use roundhouse::prelude::*;
//! # fn state() -> () {}
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # roundhouse::routes! {}
//! let built = roundhouse::app!(state()).build();
//! std::fs::write("target/openapi.json", built.openapi_json())?;
//! // built.router() is the axum router; built.routes() has the source locations.
//! # Ok(())
//! # }
//! ```

use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::handler::Handler;
use axum::http::Request;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, Route};
use tower::{Layer, Service, ServiceExt};

use crate::auth::AuthGuard;
use crate::build_error::BuildError;
use crate::doc::{self, HandlerDocs};
use crate::openapi::{Info, OpenApi};
use crate::prefix;
use crate::problem::Problem;
use crate::registry::{Method, Module, RouteRecord};

type RouterLayer = Box<dyn Fn(Router<()>) -> Router<()> + Send + Sync>;
type DocumentHook = Box<dyn Fn(&mut OpenApi) + Send + Sync>;

/// An explicit group of route modules emitted by `routes!`.
///
/// A `routes!` invocation produces one [`Module`] and a `__roundhouse_fragment()` function
/// returning this value. `...module` spreads compose fragments, and [`App::fragment`] roots the
/// app at one. The fragment owns the modules directly; there is no link-time discovery.
#[derive(Debug, Clone, Default)]
pub struct Fragment {
    roots: Vec<&'static Module>,
}

impl Fragment {
    /// Start a fragment at one module.
    pub fn new(module: &'static Module) -> Self {
        Self {
            roots: vec![module],
        }
    }

    /// Add every root of `other` to this fragment.
    pub fn include(mut self, other: Self) -> Self {
        self.roots.extend(other.roots);
        self
    }

    /// Add one module root.
    pub fn with(mut self, module: &'static Module) -> Self {
        self.roots.push(module);
        self
    }

    /// The root modules in composition order.
    pub fn roots(&self) -> &[&'static Module] {
        &self.roots
    }
}

/// A hand-written router merged with `App::merge`, plus where it was merged.
struct PendingMerge {
    router: Router<()>,
    source: crate::build_error::Source,
}

/// One entry in a built app's merge ledger.
///
/// A merged router is served but not discovered for the document, so the document is incomplete.
/// An application can assert its own migration debt with [`Built::merges`] in CI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeRecord {
    source: crate::build_error::Source,
}

impl MergeRecord {
    pub fn source(&self) -> crate::build_error::Source {
        self.source
    }

    pub fn location(&self) -> String {
        format!("{}:{}", self.source.file, self.source.line)
    }
}

/// Whether extractor rejection responses are added to the document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DocumentRejections {
    /// Add every extractor failure (`400`/`413`/`415`, auth `401`/`403`). The default.
    #[default]
    Auto,
    /// Publish only the responses the handler and its declarations supply.
    None,
}

impl From<bool> for DocumentRejections {
    fn from(enabled: bool) -> Self {
        if enabled {
            DocumentRejections::Auto
        } else {
            DocumentRejections::None
        }
    }
}

type ValidationState = (
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
    state: S,
    roots: Vec<&'static Module>,
    global_layers: Vec<RouterLayer>,
    global_layer_docs: Vec<fn() -> crate::doc::layer::LayerDoc>,
    fallback: Option<RouterLayer>,
    merges: Vec<PendingMerge>,
    info: Option<Box<dyn Fn() -> Info + Send + Sync>>,
    servers: Vec<utoipa::openapi::Server>,
    document_hooks: Vec<DocumentHook>,
    openapi_route: Option<&'static str>,
    docs_route: Option<&'static str>,
    document_rejections: DocumentRejections,
    validation_status: axum::http::StatusCode,
    validate_responses: bool,
    max_body: Option<usize>,
    #[cfg(feature = "multipart")]
    max_parts: Option<usize>,
    strict: bool,
    validation: crate::extract::Validation,
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
        self.roots.extend(fragment.roots);
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

    /// Resolve and assemble everything. Panics with the same report `try_build` returns.
    ///
    /// Duplicate routes across composed modules, state mismatches and prefix errors are
    /// discovered here. Use [`App::try_build`] in a test or check binary when CI must fail without
    /// starting the server.
    pub fn build(self) -> Built {
        let built = self.try_build().unwrap_or_else(|error| panic!("{error}"));
        for diagnostic in built.diagnostics() {
            eprintln!("{diagnostic}");
        }
        built
    }

    /// Resolve and assemble everything, returning a diagnostic instead of panicking.
    ///
    /// This is the check hook for CI: it runs the same duplicate-route, state, capture, security
    /// and document gates as [`App::build`] but never unwinds.
    pub fn try_build(self) -> Result<Built, BuildError> {
        let mut diagnostics = Vec::new();
        let serve_openapi =
            self.info.is_some() || self.openapi_route.is_some() || self.docs_route.is_some();
        let resolved = self.resolve(&mut diagnostics)?;
        self.check_reserved_paths(serve_openapi, &resolved)?;
        for merge in &self.merges {
            crate::build_error::report(
                &mut diagnostics,
                self.strict,
                crate::build_error::Diagnostic::new(
                    merge.source,
                    "a router given to `App::merge` is served but not documented; the document \
                     is marked `x-roundhouse-merged`",
                ),
            )?;
        }
        let info = self
            .info
            .as_ref()
            .map(|info| info())
            .unwrap_or_else(|| Info::new("API", "0.1.0"));
        let mut bodies = Vec::new();
        let mut parts = Vec::new();
        let mut route_bodies = BTreeMap::new();
        let mut built_document = doc::build::build_with_observations(
            info.into_utoipa(),
            &resolved,
            self.document_rejections == DocumentRejections::Auto,
            self.strict,
            &mut diagnostics,
            &mut bodies,
            &mut parts,
            &mut route_bodies,
        )?;
        if !self.merges.is_empty() {
            let extensions = utoipa::openapi::extensions::ExtensionsBuilder::new()
                .add("x-roundhouse-merged", true)
                .build();
            match &mut built_document.extensions {
                Some(existing) => existing.merge(extensions),
                None => built_document.extensions = Some(extensions),
            }
        }
        let mut document = OpenApi::from(built_document);
        if !self.servers.is_empty() {
            document
                .as_utoipa_mut()
                .servers
                .get_or_insert_with(Vec::new)
                .extend(self.servers.iter().cloned());
        }
        for hook in &self.document_hooks {
            hook(&mut document);
        }
        // The hook is part of the document too: re-run the structural checks so it cannot
        // publish a path that contradicts its own parameters.
        doc::validate_document(document.as_utoipa())
            .map_err(|message| BuildError::InvalidDocument { message })?;

        let schema_engine = if self.validation == crate::extract::Validation::Schema {
            let components = document.as_utoipa().components.clone().unwrap_or_default();
            Some(Arc::new(
                roundhouse_schema::engine::SchemaEngine::from_components_bodies_and_parts(
                    &components,
                    std::iter::empty::<(TypeId, String)>(),
                    bodies,
                    parts,
                )
                .map_err(|error| BuildError::SchemaCompilation {
                    message: error.to_string(),
                })?,
            ))
        } else {
            None
        };
        let document = Arc::new(document);

        let merge_ledger: Vec<MergeRecord> = self
            .merges
            .iter()
            .map(|merge| MergeRecord {
                source: merge.source,
            })
            .collect();

        let mut router = self.assemble(&resolved, schema_engine.as_ref(), &route_bodies)?;

        for merged in &self.merges {
            router = merge_router(router, merged.router.clone(), merged.source)?;
        }

        if serve_openapi && let Some(path) = self.openapi_route {
            let for_route = document.clone();
            router = add_get_route(
                router,
                path,
                move || {
                    let document = for_route.clone();
                    async move { axum::Json((*document).clone()) }
                },
                crate::build_error::Source::new("App::openapi_route", "<app>", 0),
            )?;
        }

        if serve_openapi && let Some(docs_path) = self.docs_route {
            let spec_path = self.openapi_route.unwrap_or("/openapi.json");
            let html = docs_html(spec_path);
            router = add_get_route(
                router,
                docs_path,
                move || {
                    let html = html.clone();
                    async move { axum::response::Html(html) }
                },
                crate::build_error::Source::new("App::docs_route", "<app>", 0),
            )?;
        }

        // A served response is part of the contract: a method mismatch uses the same Problem
        // shape as the 404 fallback instead of axum's empty default body. Applying this after
        // merges and the document routes covers every method router in the assembled app.
        router = router.method_not_allowed_fallback(|| async {
            Problem::new(
                axum::http::StatusCode::METHOD_NOT_ALLOWED,
                "Method Not Allowed",
            )
            .detail("this path does not accept the request method")
        });

        if let Some(max) = self.max_body {
            router = router.layer(axum::extract::DefaultBodyLimit::max(max));
        }

        if self.validate_responses {
            #[cfg(not(feature = "schema-engine"))]
            {
                return Err(BuildError::SchemaCompilation {
                    message: "response validation requires the `schema-engine` feature".to_string(),
                });
            }
            #[cfg(feature = "schema-engine")]
            {
                let validator = Arc::new(
                    doc::response::ResponseValidator::new(document.as_utoipa()).map_err(
                        |error| BuildError::SchemaCompilation {
                            message: error.to_string(),
                        },
                    )?,
                );
                let layer = axum::middleware::from_fn_with_state::<
                    _,
                    _,
                    (
                        axum::extract::State<Arc<doc::response::ResponseValidator>>,
                        axum::extract::Request,
                    ),
                >(validator, doc::response::validate_response);
                router = router.layer(layer);
            }
        }

        if let Some(fallback) = self.fallback {
            router = fallback(router);
        }

        #[cfg(feature = "multipart")]
        let max_parts = self.max_parts;
        #[cfg(not(feature = "multipart"))]
        let max_parts = None;

        router = router.layer(axum::middleware::from_fn_with_state(
            (
                self.validation_status,
                self.validation,
                self.max_body,
                max_parts,
                schema_engine,
            ),
            inject_validation_config,
        ));

        for layer in self.global_layers {
            router = layer(router);
        }

        Ok(Built {
            router,
            routes: resolved,
            document,
            merges: merge_ledger,
            diagnostics,
        })
    }

    /// The escape hatch: a real `axum::Router<S>`.
    ///
    /// State has already been provided inside; wrapping the ready router as a fallback service
    /// keeps the return type exactly what an axum user expects while preserving module layers and
    /// `404`/`405` semantics.
    pub fn router(self) -> Router<S> {
        let built = self.build();
        Router::<S>::new().fallback_service(built.router)
    }

    /// Build and serve on `addr`, without a graceful-shutdown signal.
    ///
    /// Use [`App::serve_with`] to stop accepting connections and let in-flight requests finish
    /// when the process receives `SIGTERM`/`SIGINT`.
    pub async fn serve(self, addr: impl Into<String>) -> std::io::Result<()> {
        self.serve_with(addr, std::future::pending()).await
    }

    /// Build and serve on `addr` until `shutdown` resolves.
    ///
    /// ```ignore
    /// app.serve_with("0.0.0.0:3000", async {
    ///     tokio::signal::ctrl_c().await.ok();
    /// }).await?;
    /// ```
    pub async fn serve_with<F>(self, addr: impl Into<String>, shutdown: F) -> std::io::Result<()>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr: String = addr.into();
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        let built = self.build();
        tracing::info!("listening on http://{}", listener.local_addr()?);
        axum::serve(listener, built.router)
            .with_graceful_shutdown(shutdown)
            .await
    }

    fn check_reserved_paths(
        &self,
        serve_openapi: bool,
        resolved: &[ResolvedRoute],
    ) -> Result<(), BuildError> {
        if !serve_openapi {
            return Ok(());
        }

        let mut reserved = Vec::new();
        if let Some(path) = self.openapi_route {
            reserved.push((path, "the generated OpenAPI document"));
        }
        if let Some(path) = self.docs_route {
            reserved.push((path, "the documentation UI"));
        }

        for (path, what) in reserved {
            let normalised = prefix::join("", path);
            if let Some(route) = resolved.iter().find(|route| {
                prefix::join("", &route.path) == normalised
                    && matches!(route.method, Method::Get | Method::Any)
            }) {
                return Err(BuildError::RouteConflict {
                    path: normalised,
                    reason: format!(
                        "{what} is served at this path; move the document route or the operation"
                    ),
                    source: route.record.source_ref(),
                });
            }
        }

        if let (Some(openapi), Some(docs)) = (self.openapi_route, self.docs_route)
            && prefix::join("", openapi) == prefix::join("", docs)
        {
            return Err(BuildError::RouteConflict {
                path: prefix::join("", openapi),
                reason: "the OpenAPI document and the documentation UI cannot share a path"
                    .to_string(),
                source: crate::build_error::Source::new("App::docs_route", "<app>", 0),
            });
        }

        Ok(())
    }

    fn resolve(
        &self,
        diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    ) -> Result<Vec<ResolvedRoute>, BuildError> {
        resolve_tree(
            &self.roots,
            self.strict,
            &self.global_layer_docs,
            Some(TypeId::of::<S>()),
            std::any::type_name::<S>(),
            self.merges.len(),
            diagnostics,
        )
    }

    fn assemble(
        &self,
        resolved: &[ResolvedRoute],
        engine: Option<&Arc<roundhouse_schema::engine::SchemaEngine>>,
        route_bodies: &BTreeMap<String, (TypeId, String)>,
    ) -> Result<Router<()>, BuildError> {
        let mut by_record: HashMap<*const RouteRecord, (String, MethodRouter<()>)> = HashMap::new();
        let mut absolute: Vec<(String, MethodRouter<()>)> = Vec::new();

        for route in resolved {
            let method_router =
                build_method_router::<S>(route, &self.state, engine, self.max_body, route_bodies)?;
            if route.record.absolute {
                absolute.push((route.path.clone(), method_router));
            } else {
                by_record.insert(
                    route.record as *const RouteRecord,
                    (route.path.clone(), method_router),
                );
            }
        }

        let mut router = Router::<()>::new();
        let mut seen: BTreeSet<*const Module> = BTreeSet::new();
        for root in &self.roots {
            if !seen.insert(*root as *const Module) {
                continue;
            }
            let (root_router, _) = build_module(root, &by_record);
            router = router.merge(root_router);
        }

        // An `absolute` route owns its full URL, so it is not part of any module router and no
        // module layer, prefix, tag or guard applies to it.
        for (path, method_router) in absolute {
            router = router.route(&path, method_router);
        }

        Ok(router.fallback(|| async { Problem::not_found("no route matched this request") }))
    }
}

fn build_module(
    module: &'static Module,
    routes: &HashMap<*const RouteRecord, (String, MethodRouter<()>)>,
) -> (Router<()>, bool) {
    let mut router = Router::<()>::new();
    let mut has_routes = false;

    for record in module.routes {
        if let Some((path, method_router)) = routes.get(&(record as *const RouteRecord)) {
            router = router.route(path, method_router.clone());
            has_routes = true;
        }
    }

    for child in module.children {
        let (child_router, child_has_routes) = build_module(child, routes);
        has_routes |= child_has_routes;
        router = router.merge(child_router);
    }

    if has_routes && let Some(layer) = module.layer {
        router = layer(router);
    }

    (router, has_routes)
}

#[allow(clippy::too_many_arguments)]
fn resolve_tree(
    roots: &[&'static Module],
    strict: bool,
    global_layer_docs: &[fn() -> crate::doc::layer::LayerDoc],
    expected_state: Option<TypeId>,
    expected_state_name: &'static str,
    merges: usize,
    diagnostics: &mut Vec<crate::build_error::Diagnostic>,
) -> Result<Vec<ResolvedRoute>, BuildError> {
    let mut state = ResolveState {
        expected_state,
        expected_state_name,
        global_layer_docs,
        seen: HashMap::new(),
        resolved: Vec::new(),
    };

    let mut seen_roots: BTreeSet<*const Module> = BTreeSet::new();
    for root in roots {
        if !seen_roots.insert(*root as *const Module) {
            continue;
        }
        let inherited = Inherited::default();
        walk_module(root, inherited, strict, diagnostics, &mut state)?;
    }

    let mut resolved = state.resolved;
    resolved.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.method.cmp(&b.method))
            .then_with(|| a.operation_id.cmp(&b.operation_id))
    });

    // `try_build` is meant to be total: probe the same matchit tree axum will build so a
    // shape conflict (e.g. `/{a}` next to `/{*rest}`) is a `BuildError`, not a panic during
    // assembly. One path may legitimately carry several methods, so insert each path once.
    let mut probe = matchit::Router::<()>::new();
    let mut previous_path: Option<&str> = None;
    for route in &resolved {
        if previous_path == Some(route.path.as_str()) {
            continue;
        }
        previous_path = Some(route.path.as_str());
        // A prefix and leaf that are valid alone can still combine into a template the router
        // cannot serve (for example more captures than matchit can name). Re-check the full
        // path before handing it to matchit, which panics past its own limit.
        if let Err(reason) = prefix::validate_template(route.path.as_str()) {
            return Err(BuildError::InvalidTemplate {
                reason,
                source: route.record.source_ref(),
            });
        }
        if let Err(error) = probe.insert(route.path.as_str(), ()) {
            return Err(BuildError::RouteConflict {
                path: route.path.clone(),
                reason: error.to_string(),
                source: route.record.source_ref(),
            });
        }
    }

    let mut operations: HashMap<&str, &'static RouteRecord> = HashMap::new();
    for route in &resolved {
        if let Some(previous) = operations.insert(route.operation_id.as_str(), route.record) {
            return Err(BuildError::DuplicateOperationId {
                operation_id: route.operation_id.clone(),
                first: previous.source_ref(),
                second: route.record.source_ref(),
            });
        }
    }

    if resolved.is_empty() && merges == 0 {
        return Err(BuildError::EmptyApp);
    }

    Ok(resolved)
}

struct ResolveState<'a> {
    expected_state: Option<TypeId>,
    expected_state_name: &'static str,
    global_layer_docs: &'a [fn() -> crate::doc::layer::LayerDoc],
    seen: HashMap<(Method, String), &'static RouteRecord>,
    resolved: Vec<ResolvedRoute>,
}

#[derive(Default, Clone)]
struct Inherited {
    prefix: String,
    tags: Vec<&'static str>,
    guards: Vec<AuthGuard>,
    hidden: bool,
    has_layer: bool,
    layer_docs: Vec<crate::doc::layer::LayerDoc>,
}

fn walk_module(
    module: &'static Module,
    inherited: Inherited,
    strict: bool,
    diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    state: &mut ResolveState<'_>,
) -> Result<bool, BuildError> {
    if let Err(reason) = prefix::validate_prefix(module.prefix) {
        return Err(BuildError::InvalidPrefix {
            prefix: module.prefix.to_string(),
            reason,
            source: module.source_ref(),
        });
    }

    let parent_prefix = inherited.prefix.trim_end_matches('/');
    let child_prefix = module.prefix.trim_end_matches('/');
    if !parent_prefix.is_empty()
        && parent_prefix != "/"
        && !child_prefix.is_empty()
        && (child_prefix == parent_prefix || child_prefix.starts_with(&format!("{parent_prefix}/")))
    {
        crate::build_error::report(
            diagnostics,
            strict,
            crate::build_error::Diagnostic::new(
                module.source_ref(),
                format!(
                    "module prefix `{}` repeats the enclosing prefix `{}`; the resolved \
                     prefix is `{}`",
                    module.prefix,
                    inherited.prefix,
                    prefix::join(&inherited.prefix, module.prefix)
                ),
            ),
        )?;
    }

    if let Some(expected) = state.expected_state {
        for guard in module.security {
            if (guard.state)() != expected {
                return Err(BuildError::SecurityStateMismatch {
                    name: guard.name.to_string(),
                    expected: state.expected_state_name,
                    actual: (guard.state_name)(),
                    source: module.source_ref(),
                });
            }
        }
    }

    let mut context = Inherited {
        prefix: prefix::join(&inherited.prefix, module.prefix),
        tags: inherited.tags,
        guards: inherited.guards,
        hidden: inherited.hidden,
        has_layer: inherited.has_layer,
        layer_docs: inherited.layer_docs,
    };
    context.tags.extend(module.tags.iter().copied());
    context.guards.extend(module.security.iter().copied());
    context.hidden |= module.hidden;
    context.has_layer |= module.layer.is_some();
    if let Some(doc) = module.doc {
        context.layer_docs.push(doc());
    }

    let mut has_routes = false;
    for record in module.routes {
        has_routes |= emit_route(record, module, &context, strict, diagnostics, state)?;
    }
    for child in module.children {
        has_routes |= walk_module(child, context.clone(), strict, diagnostics, state)?;
    }

    if !has_routes {
        crate::build_error::report(
            diagnostics,
            strict,
            crate::build_error::Diagnostic::new(
                module.source_ref(),
                format!(
                    "module `{}` declares no routes and has no routes below it",
                    module.module_path
                ),
            ),
        )?;
    }

    Ok(has_routes)
}

fn emit_route(
    record: &'static RouteRecord,
    module: &'static Module,
    context: &Inherited,
    strict: bool,
    diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    state: &mut ResolveState<'_>,
) -> Result<bool, BuildError> {
    if let Some(expected) = state.expected_state
        && let Some(actual) = (record.state)()
        && actual != expected
    {
        return Err(BuildError::StateMismatch {
            operation_id: record
                .operation_id
                .unwrap_or(record.handler_name)
                .to_string(),
            expected: (record.state_name)(),
            actual: state.expected_state_name,
            source: record.source_ref(),
        });
    }
    if let Err(reason) = prefix::validate_template(record.path) {
        return Err(BuildError::InvalidTemplate {
            reason,
            source: record.source_ref(),
        });
    }
    if let Some(expected) = state.expected_state {
        for guard in record.security {
            if (guard.state)() != expected {
                return Err(BuildError::SecurityStateMismatch {
                    name: guard.name.to_string(),
                    expected: state.expected_state_name,
                    actual: (guard.state_name)(),
                    source: record.source_ref(),
                });
            }
        }
    }

    if record.absolute {
        let mut ignored = Vec::new();
        if !context.prefix.is_empty() && context.prefix != "/" {
            ignored.push("prefix");
        }
        if !context.tags.is_empty() {
            ignored.push("tags");
        }
        if !context.guards.is_empty() {
            ignored.push("security");
        }
        if context.hidden {
            ignored.push("hidden");
        }
        if context.has_layer {
            ignored.push("layers");
        }
        if !context.layer_docs.is_empty() {
            ignored.push("layer documentation");
        }
        if !ignored.is_empty() {
            crate::build_error::report(
                diagnostics,
                strict,
                crate::build_error::Diagnostic::new(
                    record.source_ref(),
                    format!(
                        "absolute route `{}` ignores its enclosing module's {}; declare the \
                         policy on the entry instead",
                        record.operation_id.unwrap_or(record.handler_name),
                        ignored.join(", ")
                    ),
                ),
            )?;
        }
    }

    let (path, tags, auth, hidden, mut layer_docs) = if record.absolute {
        (
            prefix::join("", record.path),
            Vec::new(),
            dedup_guards(record.security.iter().copied()),
            record.hidden,
            Vec::new(),
        )
    } else {
        (
            prefix::join(&context.prefix, record.path),
            context.tags.clone(),
            dedup_guards(
                context
                    .guards
                    .iter()
                    .copied()
                    .chain(record.security.iter().copied()),
            ),
            context.hidden || record.hidden,
            context.layer_docs.clone(),
        )
    };
    layer_docs.extend(state.global_layer_docs.iter().map(|doc| doc()));

    if let Some(expected) = record.assert_path
        && path != expected
    {
        return Err(BuildError::PathAssertion {
            expected: expected.to_string(),
            actual: path,
            source: record.source_ref(),
        });
    }

    let normalised = prefix::normalise_template(&path);
    let mut captures = prefix::captures(&path);
    let total = captures.len();
    captures.sort();
    captures.dedup();
    if captures.len() != total {
        return Err(BuildError::InvalidTemplate {
            reason: format!("`{path}` declares the same capture name more than once"),
            source: record.source_ref(),
        });
    }

    let operation_id = record
        .operation_id
        .map(str::to_string)
        .unwrap_or_else(|| default_operation_id(record));
    let key = (record.method, normalised.clone());
    if let Some(previous) = state.seen.get(&key) {
        return Err(BuildError::DuplicateRoute {
            method: record.method,
            path,
            first: previous.source_ref(),
            second: record.source_ref(),
        });
    }
    for ((existing_method, existing_path), previous) in &state.seen {
        if *existing_path == normalised
            && (*existing_method == Method::Any || record.method == Method::Any)
            && *existing_method != record.method
        {
            return Err(BuildError::DuplicateRoute {
                method: record.method,
                path,
                first: previous.source_ref(),
                second: record.source_ref(),
            });
        }
    }
    state.seen.insert(key, record);

    // Keep the module in the resolved route for callers that inspect the tree; it is not used
    // for assembly, which walks `Module::routes` directly.
    let _ = module;
    state.resolved.push(ResolvedRoute {
        method: record.method,
        path,
        operation_id,
        module_path: record.module_path,
        record,
        tags,
        auth,
        hidden,
        layer_docs,
    });
    Ok(true)
}

/// Inherited guards plus the entry's own, with one guard per scheme name.
///
/// A handler that takes `Auth<A>` skips the guard for `A` at assembly time, so a scheme that is
/// declared twice is authenticated once.
fn dedup_guards(guards: impl Iterator<Item = AuthGuard>) -> Vec<AuthGuard> {
    let mut deduped: Vec<AuthGuard> = Vec::new();
    for guard in guards {
        if !deduped.iter().any(|existing| existing.name == guard.name) {
            deduped.push(guard);
        }
    }
    deduped
}

fn merge_router(
    router: Router<()>,
    merged: Router<()>,
    source: crate::build_error::Source,
) -> Result<Router<()>, BuildError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| router.merge(merged))) {
        Ok(router) => Ok(router),
        Err(payload) => Err(BuildError::RouteConflict {
            path: "<merged router>".to_string(),
            reason: format!(
                "the router given to `App::merge` collides with a declared route ({}); \
                 declare the overlapping paths through roundhouse or merge a non-overlapping router",
                panic_message(payload.as_ref())
            ),
            source,
        }),
    }
}

fn add_get_route<H, T>(
    router: Router<()>,
    path: &'static str,
    handler: H,
    source: crate::build_error::Source,
) -> Result<Router<()>, BuildError>
where
    H: Handler<T, ()> + Clone + Send + Sync + 'static,
    T: 'static,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        router.route(path, axum::routing::get(handler))
    })) {
        Ok(router) => Ok(router),
        Err(payload) => Err(BuildError::RouteConflict {
            path: path.to_string(),
            reason: format!(
                "the generated route `{path}` collides with a route already in the assembled app \
                 ({}); move the document route or declare the conflicting path through roundhouse",
                panic_message(payload.as_ref())
            ),
            source,
        }),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "route conflict".to_string()
    }
}

fn docs_html(spec_path: &str) -> String {
    format!(
        r##"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>API documentation</title>
  <link rel="stylesheet"
        href="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui.css"
        integrity="sha384-wxLW6kwyHktdDGr6Pv1zgm/VGJh99lfUbzSn6HNHBENZlCN7W602k9VkGdxuFvPn"
        crossorigin="anonymous">
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5.17.14/swagger-ui-bundle.js"
          integrity="sha384-wmyclcVGX/WhUkdkATwhaK1X1JtiNrr2EoYJ+diV3vj4v6OC5yCeSu+yW13SYJep"
          crossorigin="anonymous"></script>
  <script>
    SwaggerUIBundle({{ url: "{spec_path}", dom_id: "#swagger-ui" }});
  </script>
</body>
</html>"##
    )
}

/// `GET blog::posts::show` → `get_blog_posts_show`.
///
/// The id follows the declaration, not the URL, so moving a route under a different prefix does
/// not rename every generated client method. An explicit `operation_id` always wins.
fn default_operation_id(record: &RouteRecord) -> String {
    let mut parts = vec![record.method.as_str().to_ascii_lowercase()];
    for segment in record.module_path.split("::") {
        let segment = sanitize(segment);
        if !segment.is_empty() {
            parts.push(segment);
        }
    }
    let handler = sanitize(record.handler_name);
    if parts.last().is_none_or(|last| *last != handler) {
        parts.push(handler);
    }
    parts.join("_")
}

fn sanitize(input: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

fn build_method_router<S>(
    route: &ResolvedRoute,
    state: &S,
    engine: Option<&Arc<roundhouse_schema::engine::SchemaEngine>>,
    max_body: Option<usize>,
    route_bodies: &BTreeMap<String, (TypeId, String)>,
) -> Result<MethodRouter<()>, BuildError>
where
    S: Clone + Send + Sync + 'static,
{
    let record = route.record;
    let boxed = (record.handler)();

    let mut method_router = if (record.state)().is_some() {
        let method_router =
            boxed
                .downcast::<MethodRouter<S>>()
                .map_err(|_| BuildError::HandlerBuild {
                    operation_id: record
                        .operation_id
                        .unwrap_or(record.handler_name)
                        .to_string(),
                    message: format!(
                        "registered for state `{}` but the handler was built for another state",
                        (record.state_name)()
                    ),
                    source: record.source_ref(),
                })?;
        method_router.with_state::<()>(state.clone())
    } else {
        let method_router =
            boxed
                .downcast::<MethodRouter<()>>()
                .map_err(|_| BuildError::HandlerBuild {
                    operation_id: record
                        .operation_id
                        .unwrap_or(record.handler_name)
                        .to_string(),
                    message: "registered as stateless but the handler was built for a state"
                        .to_string(),
                    source: record.source_ref(),
                })?;
        method_router.with_state::<()>(())
    };

    if !record.validate {
        method_router = method_router.layer(axum::middleware::from_fn(skip_validation));
    }
    if record.validate
        && let Some(engine) = engine
        && let Some(type_id) = route_bodies
            .get(&route.operation_id)
            .filter(|(_, content_type)| {
                content_type == "application/json" || content_type.ends_with("+json")
            })
            .map(|(type_id, _)| *type_id)
            .or_else(|| {
                (record.body)()
                    .filter(|meta| meta.kind == crate::doc::BodyKind::Json)
                    .map(|meta| (meta.type_id)())
            })
    {
        method_router = method_router.layer(axum::middleware::from_fn_with_state(
            (engine.clone(), type_id, max_body),
            validate_json_body,
        ));
    }
    if let Some(layer) = record.layer {
        method_router = layer(method_router);
    }
    // Apply guards last so they wrap route layers: a layer that short-circuits must not be able
    // to answer before the scheme is checked. A handler that already takes `Auth<A>` performs the
    // same check during extraction, and a route-entry `security = [A]` is enforced by the
    // generated shim, so skip those guards instead of authenticating twice.
    for guard in &route.auth {
        if record.required_auth.contains(&guard.name) || record.shim_auth.contains(&guard.name) {
            continue;
        }
        method_router = (guard.apply)(method_router, state);
    }
    Ok(method_router)
}

/// Validate the raw JSON body against the operation's schema before any extractor runs.
///
/// This is what makes plain `axum::Json<T>` validated without a `Serialize` bound: the engine sees
/// the same bytes the client sent, and the request is rebuilt for the extractor. Malformed JSON is
/// also turned into a `Problem` here, so the body rejection stays one shape.
async fn validate_json_body(
    axum::extract::State((engine, type_id, max_body)): axum::extract::State<(
        Arc<roundhouse_schema::engine::SchemaEngine>,
        TypeId,
        Option<usize>,
    )>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let config = request
        .extensions()
        .get::<crate::extract::ValidationConfig>()
        .copied()
        .unwrap_or_default();
    if config.mode == crate::extract::Validation::Off
        || request
            .extensions()
            .get::<crate::extract::SkipValidation>()
            .is_some()
    {
        return next.run(request).await;
    }

    let is_json = request
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let value = value.split(';').next().unwrap_or(value).trim();
            value == "application/json" || value.ends_with("+json")
        });
    if !is_json {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let limit = max_body.unwrap_or(2 * 1024 * 1024);
    let bytes = match axum::body::to_bytes(body, limit).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Problem::from_rejection(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                "Payload Too Large",
                format!("request body exceeds {limit} bytes"),
            )
            .into_response();
        }
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return Problem::from_rejection(
                axum::http::StatusCode::BAD_REQUEST,
                "Bad Request",
                error.to_string(),
            )
            .into_response();
        }
    };
    if let Some(Err(errors)) = engine.validate_type(type_id, &value) {
        return Problem::from_validation_with_status(errors, config.status).into_response();
    }

    next.run(axum::http::Request::from_parts(
        parts,
        axum::body::Body::from(bytes),
    ))
    .await
}

async fn skip_validation(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    request
        .extensions_mut()
        .insert(crate::extract::SkipValidation);
    next.run(request).await
}

async fn inject_validation_config(
    axum::extract::State((status, mode, max_body, max_parts, engine)): axum::extract::State<
        ValidationState,
    >,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    request
        .extensions_mut()
        .insert(crate::extract::ValidationConfig { status, mode });
    #[cfg(feature = "multipart")]
    request
        .extensions_mut()
        .insert(crate::multipart::MultipartLimits {
            max_body,
            max_parts,
        });
    #[cfg(not(feature = "multipart"))]
    let _ = (max_body, max_parts);
    if let Some(engine) = engine {
        request.extensions_mut().insert(engine);
    }
    next.run(request).await
}

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
    router: Router<()>,
    routes: Vec<ResolvedRoute>,
    document: Arc<OpenApi>,
    merges: Vec<MergeRecord>,
    diagnostics: Vec<crate::build_error::Diagnostic>,
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
