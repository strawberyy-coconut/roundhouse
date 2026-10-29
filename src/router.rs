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
use utoipa::openapi::{Info, OpenApi};

use crate::build_error::BuildError;
use crate::doc::{self, HandlerDocs};
use crate::prefix;
use crate::problem::Problem;
use crate::registry::{self, Method, MountLayer, MountRecord, RouteRecord};

type RouterLayer = Box<dyn Fn(Router<()>) -> Router<()> + Send + Sync>;

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
    merges: Vec<Router<()>>,
    info: Option<Box<dyn Fn() -> Info + Send + Sync>>,
    openapi_route: Option<&'static str>,
    docs_route: Option<&'static str>,
    document_rejections: DocumentRejections,
    validation_status: axum::http::StatusCode,
    validate_responses: bool,
    max_body: Option<usize>,
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
            info: None,
            openapi_route: None,
            docs_route: None,
            document_rejections: DocumentRejections::Auto,
            validation_status: axum::http::StatusCode::BAD_REQUEST,
            validate_responses: false,
            max_body: None,
            max_parts: None,
            strict: false,
            validation: crate::extract::Validation::Generated,
        }
    }

    /// Restrict discovery to a module path (and its descendants, on `::` boundaries).
    pub fn scope(mut self, scope: &'static str) -> Self {
        self.scopes.push(scope);
        self
    }

    /// Alias for [`App::scope`], for routes supplied by a library crate.
    pub fn include(self, scope: &'static str) -> Self {
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
    pub fn merge(mut self, router: Router<()>) -> Self {
        self.merges.push(router);
        self
    }

    /// Enable `/openapi.json` and set the document info.
    pub fn openapi<F>(mut self, info: F) -> Self
    where
        F: Fn() -> Info + Send + Sync + 'static,
    {
        self.info = Some(Box::new(info));
        self.openapi_route = Some("/openapi.json");
        self
    }

    /// Choose a different path for the generated document.
    pub fn openapi_route(mut self, path: &'static str) -> Self {
        self.openapi_route = Some(path);
        self
    }

    /// Serve a Swagger UI page at `/docs` over the generated document.
    pub fn docs(mut self) -> Self {
        self.docs_route = Some("/docs");
        self
    }

    /// Serve the docs UI at a custom path.
    pub fn docs_route(mut self, path: &'static str) -> Self {
        self.docs_route = Some(path);
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
    /// This is the default for every body extractor, including multipart, where a field's
    /// `max_bytes` can only narrow it.
    pub fn max_body(mut self, bytes: usize) -> Self {
        self.max_body = Some(bytes);
        self
    }

    /// Bound the number of parts in a multipart request. Many tiny parts are a DoS axis.
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
    pub fn build(self) -> Built {
        let built = self.try_build().unwrap_or_else(|error| panic!("{error}"));
        for diagnostic in built.diagnostics() {
            eprintln!("{diagnostic}");
        }
        built
    }

    /// Resolve and assemble everything, returning a diagnostic instead of panicking.
    pub fn try_build(self) -> Result<Built, BuildError> {
        let mut diagnostics = Vec::new();
        let resolved = self.resolve(&mut diagnostics)?;
        let serve_openapi = self.info.is_some();
        let info = self
            .info
            .as_ref()
            .map(|info| info())
            .unwrap_or_else(|| Info::new("API", "0.1.0"));
        let mut bodies = Vec::new();
        let built_document = doc::build::build_with_observations(
            info,
            &resolved,
            self.document_rejections == DocumentRejections::Auto,
            self.strict,
            &mut diagnostics,
            &mut bodies,
        )?;
        let schema_engine = if self.validation == crate::extract::Validation::Schema {
            let components = built_document.components.clone().unwrap_or_default();
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
        let document = if serve_openapi || self.validate_responses {
            Some(Arc::new(built_document))
        } else {
            None
        };

        let mut router = self.assemble(&resolved)?;

        for merged in self.merges {
            router = router.merge(merged);
        }

        if serve_openapi && let (Some(document), Some(path)) = (&document, self.openapi_route) {
            let for_route = document.clone();
            router = router.route(
                path,
                axum::routing::get(move || {
                    let document = for_route.clone();
                    async move { axum::Json((*document).clone()) }
                }),
            );
        }

        if serve_openapi
            && document.is_some()
            && let Some(docs_path) = self.docs_route
        {
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

        if let Some(max) = self.max_body {
            router = router.layer(axum::extract::DefaultBodyLimit::max(max));
        }

        if self.validate_responses
            && let Some(document) = &document
        {
            #[cfg(not(feature = "schema-engine"))]
            {
                let _ = document;
                return Err(BuildError::SchemaCompilation {
                    message: "response validation requires the `schema-engine` feature".to_string(),
                });
            }
            #[cfg(feature = "schema-engine")]
            {
                let validator = Arc::new(doc::response::ResponseValidator::new(document).map_err(
                    |error| BuildError::SchemaCompilation {
                        message: error.to_string(),
                    },
                )?);
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

        router = router.layer(axum::middleware::from_fn_with_state(
            (
                self.validation_status,
                self.validation,
                self.max_body,
                self.max_parts,
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

    /// Build and serve on `addr`.
    ///
    /// `--openapi` and `--openapi-30` print the document and exit, which is what the
    /// `roundhouse openapi` command uses to dump a linked app's document. `--routes` prints the
    /// resolved table, and `--output <path>` writes either to a file instead of stdout (so a
    /// startup log on stdout cannot corrupt the JSON).
    pub async fn serve(self, addr: impl Into<String>) -> std::io::Result<()> {
        let args: Vec<String> = std::env::args().collect();
        let want_routes = args.iter().any(|arg| arg == "--routes");
        let want_30 = args.iter().any(|arg| arg == "--openapi-30");
        let want_openapi = want_30 || args.iter().any(|arg| arg == "--openapi");
        let output = output_path(&args);

        let emit = |contents: String| -> std::io::Result<()> {
            match &output {
                Some(path) => std::fs::write(path, contents),
                None => {
                    println!("{contents}");
                    Ok(())
                }
            }
        };

        if want_routes {
            return emit(self.route_listing());
        }

        if want_openapi {
            let json = if want_30 {
                self.document_30_json()?
            } else {
                self.document_json()?
            };
            return emit(json);
        }

        let addr: String = addr.into();
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        let built = self.build();
        tracing::info!("listening on http://{}", listener.local_addr()?);
        axum::serve(listener, built.router).await
    }

    /// Build the app and return the resolved route table as text.
    ///
    /// This is the explicit, non-serving path the CLI and CI should use; it does not bind a port
    /// or read `std::env::args()`.
    pub fn route_listing(self) -> String {
        let built = self.build();
        let mut out = String::new();
        for route in built.routes() {
            out.push_str(&format!(
                "{:<7} {:<40} {}\n",
                route.method,
                route.path,
                route.record.source()
            ));
        }
        out
    }

    /// Build the app and return the OpenAPI 3.1 document as pretty JSON.
    pub fn document_json(self) -> std::io::Result<String> {
        let built = self.build();
        built
            .openapi_json()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, NO_OPENAPI))
    }

    /// Build the app and return the 3.0-shaped document as pretty JSON.
    pub fn document_30_json(self) -> std::io::Result<String> {
        let built = self.build();
        built
            .openapi_30_json()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, NO_OPENAPI))
    }

    fn resolve(&self, diagnostics: &mut Vec<crate::build_error::Diagnostic>) -> Result<Vec<ResolvedRoute>, BuildError> {
        let mounts: Vec<&'static MountRecord> = inventory::iter::<registry::MountRecord>
            .into_iter()
            .filter(|mount| self.selected(mount.module_path))
            .collect();
        let records: Vec<&'static RouteRecord> = inventory::iter::<registry::RouteRecord>
            .into_iter()
            .filter(|route| self.selected(route.module_path))
            .collect();

        for mount in &mounts {
            if !prefix::captures(mount.prefix).is_empty() {
                return Err(BuildError::MountCapture {
                    prefix: mount.prefix.to_string(),
                    source: crate::build_error::Source::new(
                        mount.module_path,
                        mount.file,
                        mount.line,
                    ),
                });
            }
            let used = records
                .iter()
                .any(|route| prefix::is_ancestor(mount.module_path, route.module_path));
            if !used {
                crate::build_error::report(
                    diagnostics,
                    self.strict,
                    crate::build_error::Diagnostic::new(
                        crate::build_error::Source::new(mount.module_path, mount.file, mount.line),
                        format!("mount! `{}` has no routes below it", mount.prefix),
                    ),
                )?;
            }
        }

        let mut resolved = Vec::new();

        for scope in &self.scopes {
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
            if let Some(expected) = (record.state)()
                && expected != TypeId::of::<S>()
            {
                return Err(BuildError::StateMismatch {
                    operation_id: record.operation_id.unwrap_or(record.handler_name).to_string(),
                    expected: (record.state_name)(),
                    actual: std::any::type_name::<S>(),
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
            layer_docs.extend(self.global_layer_docs.iter().map(|doc| doc()));

            let (path, tags, security, hidden) = if record.absolute {
                (
                    prefix::join("", record.path),
                    Vec::new(),
                    Vec::new(),
                    record.hidden,
                )
            } else {
                let path =
                    prefix::join_all(ancestors.iter().map(|mount| mount.prefix), record.path);
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
            let operation_id = record
                .operation_id
                .map(str::to_string)
                .unwrap_or_else(|| default_operation_id(record, &path));
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
                layer_docs,
            });
        }

        resolved.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| a.method.cmp(&b.method))
                .then_with(|| a.operation_id.cmp(&b.operation_id))
        });

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

        if resolved.is_empty() && self.merges.is_empty() {
            return Err(BuildError::EmptyApp);
        }

        Ok(resolved)
    }

    fn selected(&self, module: &str) -> bool {
        self.scopes.is_empty()
            || self
                .scopes
                .iter()
                .any(|scope| prefix::in_scope(scope, module))
    }

    fn assemble(&self, resolved: &[ResolvedRoute]) -> Result<Router<()>, BuildError> {
        let roots = self.roots(resolved);

        let mut mounts_by_module: BTreeMap<&'static str, &'static MountRecord> = BTreeMap::new();
        for mount in inventory::iter::<registry::MountRecord>.into_iter() {
            if self.selected(mount.module_path)
                && roots
                    .iter()
                    .any(|root| prefix::is_ancestor(root, mount.module_path))
            {
                mounts_by_module.insert(mount.module_path, mount);
            }
        }

        let mut routes_by_module: BTreeMap<
            &'static str,
            Vec<(&'static RouteRecord, MethodRouter<()>)>,
        > = BTreeMap::new();
        let mut absolute: Vec<(&'static RouteRecord, MethodRouter<()>)> = Vec::new();

        for route in resolved {
            let method_router = build_method_router::<S>(route.record, &self.state)?;
            if route.record.absolute {
                absolute.push((route.record, method_router));
            } else {
                routes_by_module
                    .entry(route.module_path)
                    .or_default()
                    .push((route.record, method_router));
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
            let built = build_module(
                root,
                &all_modules,
                &children,
                &routes_by_module,
                &mounts_by_module,
            );
            let nest = mounts_by_module
                .get(root)
                .map(|mount| nest_path(mount.prefix))
                .unwrap_or_default();
            if nest.is_empty() {
                router = router.merge(built);
            } else {
                router = router.nest(&nest, built);
            }
        }

        for (record, method_router) in absolute {
            router = router.route(&prefix::join("", record.path), method_router);
        }

        Ok(router.fallback(|| async { Problem::not_found("no route matched this request") }))
    }

    fn roots(&self, resolved: &[ResolvedRoute]) -> Vec<&'static str> {
        if !self.scopes.is_empty() {
            let mut roots: Vec<&'static str> = self.scopes.clone();
            roots.sort_by_key(|scope| prefix::depth(scope));
            roots.dedup();
            // A nested scope is already covered by its ancestor; including both would duplicate
            // every route in the smaller subtree.
            let candidates = roots.clone();
            roots.retain(|scope| {
                !candidates
                    .iter()
                    .any(|other| other != scope && prefix::is_ancestor(other, scope))
            });
            return roots;
        }
        let mut roots: Vec<&'static str> = resolved
            .iter()
            .map(|route| first_segment(route.module_path))
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }
}

const NO_OPENAPI: &str = "this app did not call App::openapi(..)";

/// `--output <path>` or `--output=<path>` from the process arguments.
fn output_path(args: &[String]) -> Option<String> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(path) = arg.strip_prefix("--output=") {
            return Some(path.to_string());
        }
        if arg == "--output" {
            return args.next().cloned();
        }
    }
    None
}

fn docs_html(spec_path: &str) -> String {    format!(
        r##"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>API documentation</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css">
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
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

/// `GET /blog/posts/{id}` → `get_blog_posts_id`.
///
/// A path made only of captures (or `/`) is not unique across modules, so those also include the
/// module's last segment and the handler name. An explicit `operation_id` always wins.
fn default_operation_id(record: &RouteRecord, path: &str) -> String {
    let mut parts = vec![record.method.as_str().to_ascii_lowercase()];
    let mut literal = false;
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        if let Some(name) = segment
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
        {
            parts.push(sanitize(name.strip_prefix('*').unwrap_or(name)));
        } else {
            parts.push(sanitize(segment));
            literal = true;
        }
    }
    if !literal {
        parts.push(sanitize(
            record
                .module_path
                .rsplit("::")
                .next()
                .unwrap_or(record.module_path),
        ));
        parts.push(sanitize(record.handler_name));
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

fn nest_path(prefix: &str) -> String {
    let joined = prefix::join("", prefix);
    if joined == "/" { String::new() } else { joined }
}

fn build_module(
    module: &'static str,
    modules: &BTreeSet<&'static str>,
    children: &BTreeMap<&'static str, Vec<&'static str>>,
    routes: &BTreeMap<&'static str, Vec<(&'static RouteRecord, MethodRouter<()>)>>,
    mounts: &BTreeMap<&'static str, &'static MountRecord>,
) -> Router<()> {
    let mut router = Router::<()>::new();

    if let Some(items) = routes.get(module) {
        for (record, method_router) in items {
            router = router.route(record.path, method_router.clone());
        }
    }

    if let Some(items) = children.get(module) {
        for child in items {
            if !modules.contains(child) {
                continue;
            }
            let child_router = build_module(child, modules, children, routes, mounts);
            let nest = mounts
                .get(child)
                .map(|mount| nest_path(mount.prefix))
                .unwrap_or_default();
            if nest.is_empty() {
                router = router.merge(child_router);
            } else {
                router = router.nest(&nest, child_router);
            }
        }
    }

    if let Some(mount) = mounts.get(module)
        && let Some(layer) = mount.layer
    {
        router = apply_mount_layer(router, layer);
    }

    router
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
        let method_router = boxed.downcast::<MethodRouter<S>>().map_err(|_| {
            BuildError::HandlerBuild {
                operation_id: record.operation_id.unwrap_or(record.handler_name).to_string(),
                message: format!(
                    "registered for state `{}` but the handler was built for another state",
                    (record.state_name)()
                ),
                source: record.source_ref(),
            }
        })?;
        method_router.with_state::<()>(state.clone())
    } else {
        let method_router = boxed
            .downcast::<MethodRouter<()>>()
            .map_err(|_| BuildError::HandlerBuild {
                operation_id: record.operation_id.unwrap_or(record.handler_name).to_string(),
                message: "registered as stateless but the handler was built for a state".to_string(),
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
    request
        .extensions_mut()
        .insert(crate::multipart::MultipartLimits {
            max_body,
            max_parts,
        });
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
    document: Option<Arc<OpenApi>>,
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

    /// The generated document, when `App::openapi(..)` was called.
    pub fn document(&self) -> Option<&OpenApi> {
        self.document.as_deref()
    }

    /// Non-fatal observations collected while resolving and documenting the app.
    pub fn diagnostics(&self) -> &[crate::build_error::Diagnostic] {
        &self.diagnostics
    }

    pub fn openapi_json(&self) -> Option<String> {
        self.document.as_ref().map(|document| {
            document
                .to_pretty_json()
                .expect("OpenAPI document serializes")
        })
    }

    /// A 3.0.3-shaped JSON document.
    pub fn openapi_30(&self) -> Option<serde_json::Value> {
        self.document
            .as_ref()
            .map(|document| doc::downgrade::to_openapi_30(document))
    }

    pub fn openapi_30_json(&self) -> Option<String> {
        self.openapi_30()
            .map(|value| serde_json::to_string_pretty(&value).expect("JSON serializes"))
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
