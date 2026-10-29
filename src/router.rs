//! `App`, route resolution, and router assembly.

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

use crate::build_error::BuildError;
use crate::doc::{self, HandlerDocs};
use crate::openapi::{Info, OpenApi};
use crate::prefix;
use crate::problem::Problem;
use crate::registry::{self, Method, MountLayer, MountRecord, RouteRecord};

type RouterLayer = Box<dyn Fn(Router<()>) -> Router<()> + Send + Sync>;
type DocumentHook = Box<dyn Fn(&mut OpenApi) + Send + Sync>;

/// A typed handle to a module's route declarations, exported by `mount!` as `MOUNT`.
///
/// `App::scope`, `App::include` and `App::mount` accept this instead of a raw module path, so a
/// partial move across a crate boundary fails to compile rather than silently selecting fewer
/// routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountHandle {
    scope: &'static str,
    prefix: &'static str,
}

impl MountHandle {
    pub const fn new(scope: &'static str, prefix: &'static str) -> Self {
        Self { scope, prefix }
    }

    pub const fn scope(self) -> &'static str {
        self.scope
    }

    pub const fn prefix(self) -> &'static str {
        self.prefix
    }
}

/// A module scope for [`App::scope`], [`App::include`] and [`App::mount`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope(&'static str);

impl Scope {
    pub const fn module_path(self) -> &'static str {
        self.0
    }
}

impl From<&'static str> for Scope {
    fn from(scope: &'static str) -> Self {
        Self(scope)
    }
}

impl From<MountHandle> for Scope {
    fn from(handle: MountHandle) -> Self {
        Self(handle.scope)
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

/// `App::mount(prefix, scope)`: an additional prefix for routes declared under `scope`.
#[derive(Clone, Copy)]
struct MountBinding {
    prefix: &'static str,
    scope: &'static str,
    source: crate::build_error::Source,
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
    scopes: Vec<&'static str>,
    global_layers: Vec<RouterLayer>,
    global_layer_docs: Vec<fn() -> crate::doc::layer::LayerDoc>,
    fallback: Option<RouterLayer>,
    merges: Vec<PendingMerge>,
    mounts: Vec<MountBinding>,
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
            scopes: Vec::new(),
            global_layers: Vec::new(),
            global_layer_docs: Vec::new(),
            fallback: None,
            merges: Vec::new(),
            mounts: Vec::new(),
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
            validation: crate::extract::Validation::Generated,
        }
    }

    /// Restrict discovery to a module path (and its descendants, on `::` boundaries).
    ///
    /// Accepts a `mount!` handle (`other_crate::routes::MOUNT`) as well as a string module path.
    pub fn scope(mut self, scope: impl Into<Scope>) -> Self {
        self.scopes.push(scope.into().module_path());
        self
    }

    /// Alias for [`App::scope`], for routes supplied by a library crate.
    pub fn include(self, scope: impl Into<Scope>) -> Self {
        self.scope(scope)
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

    /// Serve routes declared under `scope` at `prefix` as well as their normal location.
    ///
    /// This is the composition path for a route table owned by another crate, and for serving one
    /// table at more than one versioned prefix. The selected declarations go through the same
    /// state, conflict, capture, security and response checks as every other route.
    #[track_caller]
    pub fn mount(mut self, prefix: &'static str, scope: impl Into<Scope>) -> Self {
        let location = std::panic::Location::caller();
        self.mounts.push(MountBinding {
            prefix,
            scope: scope.into().module_path(),
            source: crate::build_error::Source::new("App::mount", location.file(), location.line()),
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

    /// Choose the enforcer: generated checks (default), the emitted JSON Schema, or none.
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
    /// Duplicate routes, state mismatches and mount errors are discovered here, not by
    /// `cargo check`: the route tables are link-time records. Use [`App::try_build`] in a test or
    /// check binary when CI must fail without starting the server.
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
        let resolved = self.resolve(&mut diagnostics)?;
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
        let serve_openapi =
            self.info.is_some() || self.openapi_route.is_some() || self.docs_route.is_some();
        let info = self
            .info
            .as_ref()
            .map(|info| info())
            .unwrap_or_else(|| Info::new("API", "0.1.0"));
        let mut bodies = Vec::new();
        let mut built_document = doc::build::build_with_observations(
            info.into_utoipa(),
            &resolved,
            self.document_rejections == DocumentRejections::Auto,
            self.strict,
            &mut diagnostics,
            &mut bodies,
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
            let types = inventory::iter::<registry::ComponentTypeRecord>
                .into_iter()
                .map(|record| ((record.type_id)(), doc::record_component_name(record)));
            Some(Arc::new(
                roundhouse_schema::engine::SchemaEngine::from_components_and_bodies(
                    &components,
                    types,
                    bodies,
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

        let mut router = self.assemble(&resolved)?;

        for merged in self.merges {
            router = router.merge(merged.router);
        }

        if serve_openapi && let Some(path) = self.openapi_route {
            let for_route = document.clone();
            router = router.route(
                path,
                axum::routing::get(move || {
                    let document = for_route.clone();
                    async move { axum::Json((*document).clone()) }
                }),
            );
        }

        if serve_openapi && let Some(docs_path) = self.docs_route {
            let spec_path = self.openapi_route.unwrap_or("/openapi.json");
            let html = docs_html(spec_path);
            router = router.route(
                docs_path,
                axum::routing::get(move || {
                    let html = html.clone();
                    async move { axum::response::Html(html) }
                }),
            );
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

    fn resolve(
        &self,
        diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    ) -> Result<Vec<ResolvedRoute>, BuildError> {
        resolve_routes(
            &self.scopes,
            &self.mounts,
            self.strict,
            &self.global_layer_docs,
            Some(TypeId::of::<S>()),
            std::any::type_name::<S>(),
            self.merges.len(),
            diagnostics,
        )
    }

    fn assemble(&self, resolved: &[ResolvedRoute]) -> Result<Router<()>, BuildError> {
        let roots = self.roots(resolved);

        let mut mounts_by_module: BTreeMap<&'static str, &'static MountRecord> = BTreeMap::new();
        for mount in inventory::iter::<registry::MountRecord>.into_iter() {
            if roots
                .iter()
                .any(|root| prefix::is_ancestor(root, mount.module_path))
            {
                mounts_by_module.insert(mount.module_path, mount);
            }
        }

        let mut routes_by_module: BTreeMap<&'static str, Vec<(String, MethodRouter<()>)>> =
            BTreeMap::new();
        let mut absolute: Vec<(&'static RouteRecord, MethodRouter<()>)> = Vec::new();

        for route in resolved {
            let method_router = build_method_router::<S>(route.record, &self.state)?;
            if route.record.absolute {
                absolute.push((route.record, method_router));
            } else {
                routes_by_module
                    .entry(route.module_path)
                    .or_default()
                    .push((route.path.clone(), method_router));
            }
        }

        let mut all_modules: BTreeSet<&'static str> = BTreeSet::new();
        for route in resolved {
            if route.record.absolute {
                continue;
            }
            let root = roots
                .iter()
                .copied()
                .filter(|root| prefix::is_ancestor(root, route.module_path))
                .max_by_key(|root| prefix::depth(root))
                .unwrap_or_else(|| first_segment(route.module_path));
            let mut module = route.module_path;
            loop {
                all_modules.insert(module);
                if module == root {
                    break;
                }
                match module.rsplit_once("::") {
                    Some((parent, _)) => module = parent,
                    None => break,
                }
            }
        }
        for module in mounts_by_module.keys() {
            all_modules.insert(module);
        }

        let mut children: BTreeMap<&'static str, Vec<&'static str>> = BTreeMap::new();
        for module in &all_modules {
            if let Some((parent, _)) = module.rsplit_once("::")
                && all_modules.contains(parent)
            {
                children.entry(parent).or_default().push(module);
            }
        }

        let mut router = Router::<()>::new();

        let mut built_roots: Vec<&'static str> = Vec::new();
        for root in &roots {
            if all_modules.contains(root) || mounts_by_module.contains_key(root) {
                built_roots.push(root);
            }
        }
        for root in built_roots {
            // Every module registers its routes at the full resolved path, so the tree is flat:
            // a root and its descendants are one router merged into the app.
            router = router.merge(build_module(
                root,
                &all_modules,
                &children,
                &routes_by_module,
                &mounts_by_module,
            ));
        }

        for (record, method_router) in absolute {
            router = router.route(&prefix::join("", record.path), method_router);
        }

        Ok(router.fallback(|| async { Problem::not_found("no route matched this request") }))
    }

    fn roots(&self, resolved: &[ResolvedRoute]) -> Vec<&'static str> {
        // The tree is flat and already filtered to this app's scopes; rooting it at the first
        // module segment lets ancestor mounts (including ones above a `scope`) contribute their
        // layers to the selected routes without pulling in any unselected route.
        let mut roots: Vec<&'static str> = resolved
            .iter()
            .map(|route| first_segment(route.module_path))
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }
}

fn selected(scopes: &[&'static str], module: &str) -> bool {
    scopes.is_empty() || scopes.iter().any(|scope| prefix::in_scope(scope, module))
}

#[allow(clippy::too_many_arguments)]
fn resolve_routes(
    scopes: &[&'static str],
    app_mounts: &[MountBinding],
    strict: bool,
    global_layer_docs: &[fn() -> crate::doc::layer::LayerDoc],
    expected_state: Option<TypeId>,
    expected_state_name: &'static str,
    merges: usize,
    diagnostics: &mut Vec<crate::build_error::Diagnostic>,
) -> Result<Vec<ResolvedRoute>, BuildError> {
    let selected_by_mount = |module: &str| {
        app_mounts
            .iter()
            .any(|mount| prefix::in_scope(mount.scope, module))
    };
    let records: Vec<&'static RouteRecord> = inventory::iter::<registry::RouteRecord>
        .into_iter()
        .filter(|route| selected(scopes, route.module_path) || selected_by_mount(route.module_path))
        .collect();
    // A scope selects a subtree, not a new root. Mounts above the scope still provide the
    // structural prefix, tags, security and layers for every route inside it.
    let mounts: Vec<&'static MountRecord> = inventory::iter::<registry::MountRecord>
        .into_iter()
        .filter(|mount| {
            selected(scopes, mount.module_path)
                || records
                    .iter()
                    .any(|route| prefix::is_ancestor(mount.module_path, route.module_path))
        })
        .collect();

    for mount in &mounts {
        if let Err(reason) = prefix::validate_mount_prefix(mount.prefix) {
            return Err(BuildError::InvalidMountPrefix {
                prefix: mount.prefix.to_string(),
                reason,
                source: crate::build_error::Source::new(mount.module_path, mount.file, mount.line),
            });
        }
        let used = records
            .iter()
            .any(|route| prefix::is_ancestor(mount.module_path, route.module_path));
        if !used && selected(scopes, mount.module_path) {
            crate::build_error::report(
                diagnostics,
                strict,
                crate::build_error::Diagnostic::new(
                    crate::build_error::Source::new(mount.module_path, mount.file, mount.line),
                    format!("mount! `{}` has no routes below it", mount.prefix),
                ),
            )?;
        }
    }

    let mut resolved = Vec::new();

    for mount in app_mounts {
        if let Err(reason) = prefix::validate_mount_prefix(mount.prefix) {
            return Err(BuildError::InvalidMountPrefix {
                prefix: mount.prefix.to_string(),
                reason,
                source: mount.source,
            });
        }
        if !records
            .iter()
            .any(|route| prefix::in_scope(mount.scope, route.module_path))
        {
            return Err(BuildError::EmptyScope { scope: mount.scope });
        }
    }

    for scope in scopes {
        let matches = inventory::iter::<registry::RouteRecord>
            .into_iter()
            .any(|route| prefix::is_ancestor(scope, route.module_path))
            || inventory::iter::<registry::MountRecord>
                .into_iter()
                .any(|mount| prefix::is_ancestor(scope, mount.module_path));
        if !matches {
            return Err(BuildError::EmptyScope { scope });
        }
    }

    let mut seen: HashMap<(Method, String), &'static RouteRecord> = HashMap::new();

    for record in records {
        if let Some(expected) = expected_state
            && let Some(actual) = (record.state)()
            && actual != expected
        {
            return Err(BuildError::StateMismatch {
                operation_id: record
                    .operation_id
                    .unwrap_or(record.handler_name)
                    .to_string(),
                expected: (record.state_name)(),
                actual: expected_state_name,
                source: record.source_ref(),
            });
        }
        if let Err(reason) = prefix::validate_template(record.path) {
            return Err(BuildError::InvalidTemplate {
                reason,
                source: record.source_ref(),
            });
        }

        let ancestors: Vec<&'static MountRecord> = mounts
            .iter()
            .copied()
            .filter(|mount| prefix::is_ancestor(mount.module_path, record.module_path))
            .collect();

        let mut ancestors = ancestors;
        ancestors.sort_by_key(|mount| prefix::depth(mount.module_path));

        let mut layer_docs: Vec<crate::doc::layer::LayerDoc> = if record.absolute {
            Vec::new()
        } else {
            ancestors
                .iter()
                .filter_map(|mount| mount.doc)
                .map(|doc| doc())
                .collect()
        };
        layer_docs.extend(global_layer_docs.iter().map(|doc| doc()));

        // The assertion describes the declaration's own prefix chain, before any
        // `App::mount` binding. A library can therefore assert its public URL and still be
        // mounted at a versioned prefix by an application.
        let declared_path = if record.absolute {
            prefix::join("", record.path)
        } else {
            prefix::join_all(ancestors.iter().map(|mount| mount.prefix), record.path)
        };
        if let Some(expected) = record.assert_path
            && declared_path != expected
        {
            return Err(BuildError::PathAssertion {
                expected: expected.to_string(),
                actual: declared_path,
                source: record.source_ref(),
            });
        }

        let normal = selected(scopes, record.module_path);
        let mut bindings: Vec<Option<MountBinding>> = Vec::new();
        if record.absolute {
            // `absolute` means the entry owns its full path, so it is emitted once even if more
            // than one app mount covers its scope.
            bindings.push(None);
        } else {
            if normal {
                bindings.push(None);
            }
            bindings.extend(
                app_mounts
                    .iter()
                    .copied()
                    .filter(|mount| prefix::in_scope(mount.scope, record.module_path))
                    .map(Some),
            );
        }

        for binding in bindings {
            let (path, tags, security, hidden) = if record.absolute {
                (
                    prefix::join("", record.path),
                    Vec::new(),
                    Vec::new(),
                    record.hidden,
                )
            } else {
                let path = match binding {
                    Some(mount) => prefix::join_all(
                        std::iter::once(mount.prefix)
                            .chain(ancestors.iter().map(|mount| mount.prefix)),
                        record.path,
                    ),
                    None => {
                        prefix::join_all(ancestors.iter().map(|mount| mount.prefix), record.path)
                    }
                };
                let tags = ancestors
                    .iter()
                    .flat_map(|mount| mount.tags.iter().copied())
                    .collect();
                let security = ancestors
                    .iter()
                    .flat_map(|mount| mount.security.iter().copied())
                    .collect();
                let hidden = record.hidden || ancestors.iter().any(|mount| mount.hidden);
                (path, tags, security, hidden)
            };

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
            let mount_prefix = binding.as_ref().map(|mount| mount.prefix);
            let operation_id = record
                .operation_id
                .map(str::to_string)
                .unwrap_or_else(|| default_operation_id(record, mount_prefix));
            let key = (record.method, normalised.clone());
            if let Some(previous) = seen.get(&key) {
                return Err(BuildError::DuplicateRoute {
                    method: record.method,
                    path,
                    first: previous.source_ref(),
                    second: record.source_ref(),
                });
            }
            for ((existing_method, existing_path), previous) in &seen {
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
            seen.insert(key, record);

            resolved.push(ResolvedRoute {
                method: record.method,
                path,
                operation_id,
                module_path: record.module_path,
                record,
                tags,
                security,
                hidden,
                layer_docs: layer_docs.clone(),
            });
        }
    }

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
        // A mount and leaf that are valid alone can still combine into a template the router
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

fn first_segment(module: &'static str) -> &'static str {
    module.split("::").next().unwrap_or(module)
}

/// `GET blog::posts::show` → `get_blog_posts_show`.
///
/// The id follows the declaration, not the URL, so moving a route under a different prefix does
/// not rename every generated client method. A second binding of the same table (via
/// `App::mount`) appends that binding's prefix, because one OpenAPI document cannot contain two
/// operations with the same id. An explicit `operation_id` always wins.
fn default_operation_id(record: &RouteRecord, mount_prefix: Option<&str>) -> String {
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
    if let Some(prefix) = mount_prefix {
        let prefix = sanitize(prefix);
        if !prefix.is_empty() {
            parts.push(prefix);
        }
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

fn build_module(
    module: &'static str,
    modules: &BTreeSet<&'static str>,
    children: &BTreeMap<&'static str, Vec<&'static str>>,
    routes: &BTreeMap<&'static str, Vec<(String, MethodRouter<()>)>>,
    mounts: &BTreeMap<&'static str, &'static MountRecord>,
) -> Router<()> {
    let (router, _) = build_module_inner(module, modules, children, routes, mounts);
    router
}

/// Build one module router, returning whether it contains any routes.
///
/// The flag lets `build_module` skip a `route_layer` on a module whose routes were all filtered
/// out; axum panics when a route layer is added to a router with no routes.
fn build_module_inner(
    module: &'static str,
    modules: &BTreeSet<&'static str>,
    children: &BTreeMap<&'static str, Vec<&'static str>>,
    routes: &BTreeMap<&'static str, Vec<(String, MethodRouter<()>)>>,
    mounts: &BTreeMap<&'static str, &'static MountRecord>,
) -> (Router<()>, bool) {
    let mut router = Router::<()>::new();
    let mut has_routes = false;

    if let Some(items) = routes.get(module) {
        for (path, method_router) in items {
            router = router.route(path, method_router.clone());
            has_routes = true;
        }
    }

    if let Some(items) = children.get(module) {
        for child in items {
            if !modules.contains(child) {
                continue;
            }
            let (child_router, child_has_routes) =
                build_module_inner(child, modules, children, routes, mounts);
            has_routes |= child_has_routes;
            router = router.merge(child_router);
        }
    }

    if has_routes
        && let Some(mount) = mounts.get(module)
        && let Some(layer) = mount.layer
    {
        router = apply_mount_layer(router, layer);
    }

    (router, has_routes)
}

fn apply_mount_layer(router: Router<()>, layer: MountLayer) -> Router<()> {
    layer(router)
}

fn build_method_router<S>(
    record: &'static RouteRecord,
    state: &S,
) -> Result<MethodRouter<()>, BuildError>
where
    S: Clone + Send + Sync + 'static,
{
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
    if let Some(layer) = record.layer {
        method_router = layer(method_router);
    }
    Ok(method_router)
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
    pub security: Vec<&'static str>,
    pub hidden: bool,
    pub layer_docs: Vec<crate::doc::layer::LayerDoc>,
}

impl ResolvedRoute {
    pub fn docs(&self) -> HandlerDocs {
        let mut components = utoipa::openapi::schema::Components::new();
        (self.record.docs)(&mut components)
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
