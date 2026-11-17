use std::any::TypeId;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
#[cfg(feature = "contract")]
use std::sync::OnceLock;

use axum::Router;
use axum::routing::MethodRouter;

use crate::build_error::BuildError;
#[cfg(feature = "contract")]
use crate::doc;
#[cfg(feature = "contract")]
use crate::openapi::{Info, OpenApi};
#[cfg(feature = "contract-runtime")]
use crate::prefix;
use crate::problem::Problem;
#[cfg(feature = "contract-runtime")]
use crate::registry::Method;
use crate::registry::{Module, RouteRecord};

#[cfg(feature = "contract")]
use super::app::DocumentHook;
use super::app::{App, ValidationDefaults};
#[cfg(feature = "contract-runtime")]
use super::assemble::add_get_route;
#[cfg(all(feature = "swagger", feature = "contract-runtime"))]
use super::assemble::add_swagger_routes;
#[cfg(feature = "contract-runtime")]
use super::assemble::docs_html;
use super::assemble::{build_method_router, build_module, inject_validation_config, merge_router};
use super::built::{Built, ResolvedRoute};
use super::fragment::MergeRecord;
use super::resolve::resolve_tree;

/// Build the OpenAPI document from resolved routes, apply servers and document hooks, and run
/// the structural document checks.
///
/// This is the shared half of `App::build` used by the state-less contract builder. It does not
/// know about application state, routers or merges.
#[cfg(feature = "contract")]
pub(crate) struct DocumentConfig<'a> {
    pub(crate) include_rejections: bool,
    pub(crate) strict: bool,
    pub(crate) allow_undeclared_success: bool,
    pub(crate) inference: &'a crate::doc::inference::Inference,
    pub(crate) response_defaults: &'a std::collections::BTreeMap<u16, String>,
    pub(crate) registrations: &'a [fn(&mut dyn crate::doc::SchemaContext)],
    pub(crate) servers: &'a [utoipa::openapi::Server],
    pub(crate) hooks: &'a [DocumentHook],
}

#[cfg(feature = "contract")]
pub(crate) fn build_openapi(
    info: Info,
    resolved: &[ResolvedRoute],
    config: DocumentConfig<'_>,
) -> Result<(OpenApi, doc::build::Observations), BuildError> {
    let mut observations = doc::build::Observations::default();
    let built_document = doc::build::build_with_observations_and_inference(
        info.into_utoipa(),
        resolved,
        config.include_rejections,
        config.strict,
        config.allow_undeclared_success,
        config.inference,
        config.response_defaults,
        config.registrations,
        &mut observations,
    )?;

    let mut document = OpenApi::from(built_document);
    if !config.servers.is_empty() {
        document
            .as_utoipa_mut()
            .servers
            .get_or_insert_with(Vec::new)
            .extend(config.servers.iter().cloned());
    }
    for hook in config.hooks {
        hook(&mut document);
    }
    // The hook is part of the document too: re-run the structural checks so it cannot publish a
    // path that contradicts its own parameters.
    doc::validate_document(document.as_utoipa())
        .map_err(|message| BuildError::InvalidDocument { message })?;
    Ok((document, observations))
}

impl<S> App<S>
where
    S: Clone + Send + Sync + 'static,
{
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
    #[allow(unused_mut)]
    pub fn try_build(mut self) -> Result<Built, BuildError> {
        let mut diagnostics = Vec::new();
        #[cfg(not(feature = "contract"))]
        {
            #[cfg(feature = "contract-runtime")]
            let has_contract = self.contract_bundle.is_some();
            #[cfg(not(feature = "contract-runtime"))]
            let has_contract = false;
            if self.validation != crate::extract::Validation::Off && !has_contract {
                return Err(BuildError::MissingContract);
            }
        }
        let serve_openapi = self.serves_openapi();
        let resolved = self.resolve(&mut diagnostics)?;
        self.check_reserved_paths(serve_openapi, &resolved)?;
        self.report_merges(&mut diagnostics)?;

        #[cfg(feature = "contract")]
        if self.contract_bundle.is_none() {
            let (bundle, contract_diagnostics) = self.build_contract_bundle()?;
            self.contract_bundle = Some(bundle);
            diagnostics.extend(contract_diagnostics);
        }
        #[cfg(feature = "contract-runtime")]
        if let Some(bundle) = self.contract_bundle {
            bundle
                .validate()
                .map_err(|message| BuildError::InvalidContract { message })?;
        }
        #[cfg(feature = "contract")]
        let (schema_engine, contract_openapi) = {
            let bundle = self
                .contract_bundle
                .expect("a contract bundle is installed before assembly");
            let schema_engine = self.compile_contract_schema_engine(bundle)?;
            (schema_engine, Some(bundle.openapi))
        };
        #[cfg(all(feature = "contract-runtime", not(feature = "contract")))]
        let (schema_engine, contract_openapi) = if let Some(bundle) = self.contract_bundle {
            (
                self.compile_contract_schema_engine(bundle)?,
                Some(bundle.openapi),
            )
        } else {
            (None, None)
        };
        #[cfg(not(feature = "contract-runtime"))]
        let (schema_engine, contract_openapi): (
            Option<Arc<roundhouse_schema::engine::SchemaEngine>>,
            Option<&'static str>,
        ) = (None, None);
        #[cfg(not(feature = "contract-runtime"))]
        let _ = contract_openapi;

        let merge_ledger = self.merge_ledger();
        #[cfg(feature = "contract")]
        let document = Arc::new(OnceLock::new());
        #[cfg(feature = "contract")]
        let router = self.assemble_app(&resolved, schema_engine)?;
        #[cfg(all(feature = "contract-runtime", not(feature = "contract")))]
        let router = self.assemble_app(&resolved, schema_engine)?;
        #[cfg(not(feature = "contract-runtime"))]
        let router = self.assemble_app(&resolved, schema_engine)?;

        Ok(Built {
            router,
            routes: resolved,
            #[cfg(feature = "contract")]
            document,
            #[cfg(feature = "contract-runtime")]
            contract_openapi,
            merges: merge_ledger,
            diagnostics,
        })
    }

    fn serves_openapi(&self) -> bool {
        #[cfg(not(feature = "contract-runtime"))]
        {
            return false;
        }
        #[cfg(all(feature = "contract-runtime", not(feature = "contract")))]
        {
            #[cfg(feature = "swagger")]
            let swagger = self.swagger_route.is_some();
            #[cfg(not(feature = "swagger"))]
            let swagger = false;
            self.contract_bundle.is_some()
                && (self.openapi_route.is_some() || self.docs_route.is_some() || swagger)
        }
        #[cfg(feature = "contract")]
        {
            #[cfg(feature = "swagger")]
            let swagger = self.swagger_route.is_some();
            #[cfg(not(feature = "swagger"))]
            let swagger = false;

            self.info.is_some()
                || self.openapi_route.is_some()
                || self.docs_route.is_some()
                || swagger
        }
    }

    fn report_merges(
        &self,
        diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    ) -> Result<(), BuildError> {
        for merge in &self.merges {
            if merge.reason.is_some() {
                continue;
            }
            crate::build_error::report(
                diagnostics,
                self.strict,
                crate::build_error::Diagnostic::new(
                    merge.source,
                    "a router given to `App::merge` is served but not documented; use \
                     `App::merge_undocumented(router, reason)` to acknowledge it",
                ),
            )?;
        }
        Ok(())
    }

    /// Build a contract from the app's own declarations.
    ///
    /// Tests and apps that do not pass an artifact still get the same strict contract checks,
    /// but the runtime no longer assembles a separate document or schema engine.
    #[cfg(feature = "contract")]
    fn build_contract_bundle(
        &mut self,
    ) -> Result<(crate::ContractBundle, Vec<crate::build_error::Diagnostic>), BuildError> {
        let info = self
            .info
            .as_ref()
            .map(|info| info())
            .unwrap_or_else(|| Info::new("API", "0.1.0"));
        let mut contract = crate::Contract::<S>::new(info);
        for root in &self.roots {
            contract = contract.include(root);
        }
        contract = contract.servers(std::mem::take(&mut self.servers));
        for hook in std::mem::take(&mut self.document_hooks) {
            contract = contract.document(move |document| hook(document));
        }
        for doc in std::mem::take(&mut self.global_layer_docs) {
            contract = contract.layer_doc(doc);
        }
        contract = contract.document_rejections(self.document_rejections);
        contract = if self.strict {
            contract.strict()
        } else {
            contract.permissive().allow_undeclared_success(true)
        };

        let built = contract.build()?;
        let diagnostics = built.diagnostics().to_vec();
        let mut openapi: serde_json::Value =
            serde_json::from_str(&built.openapi_json()).map_err(|error| {
                BuildError::InvalidDocument {
                    message: error.to_string(),
                }
            })?;
        if !self.merges.is_empty() {
            openapi["x-roundhouse-merged"] = serde_json::json!(true);
        }
        let openapi = serde_json::to_string_pretty(&openapi)
            .expect("the assembled OpenAPI document serializes");
        let contract_json =
            built
                .contract_json()
                .map_err(|error| BuildError::SchemaCompilation {
                    message: error.to_string(),
                })?;
        Ok((
            crate::ContractBundle::new(
                Box::leak(contract_json.into_boxed_str()),
                Box::leak(openapi.into_boxed_str()),
            ),
            diagnostics,
        ))
    }

    #[cfg(feature = "contract-runtime")]
    fn compile_contract_schema_engine(
        &self,
        bundle: crate::ContractBundle,
    ) -> Result<Option<Arc<roundhouse_schema::engine::SchemaEngine>>, BuildError> {
        if self.validation != crate::extract::Validation::Schema {
            return Ok(None);
        }
        let engine = roundhouse_schema::engine::SchemaEngine::from_contract_json(bundle.contract)
            .map_err(|error| BuildError::SchemaCompilation {
            message: error.to_string(),
        })?;
        Ok(Some(Arc::new(engine)))
    }

    fn merge_ledger(&self) -> Vec<MergeRecord> {
        self.merges
            .iter()
            .map(|merge| MergeRecord {
                source: merge.source,
                reason: merge.reason,
            })
            .collect()
    }

    #[cfg(feature = "contract")]
    fn assemble_app(
        self,
        resolved: &[ResolvedRoute],
        schema_engine: Option<Arc<roundhouse_schema::engine::SchemaEngine>>,
    ) -> Result<Router<()>, BuildError> {
        let serve_openapi = self.serves_openapi();
        let mut router = self.assemble(resolved, schema_engine.as_ref())?;

        for merged in &self.merges {
            router = merge_router(router, merged.router.clone(), merged.source)?;
        }

        if serve_openapi
            && let Some(path) = self.openapi_route
            && let Some(bundle) = self.contract_bundle
        {
            let body = bundle.openapi;
            router = add_get_route(
                router,
                path,
                move || async move {
                    let mut response = axum::response::Response::new(axum::body::Body::from(body));
                    response.headers_mut().insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    response
                },
                crate::build_error::Source::new("App::openapi_route", "<app>", 0),
            )?;
        }

        if serve_openapi && let Some(docs_path) = self.docs_route {
            let spec_path = self.openapi_route.unwrap_or("/openapi.json");
            #[cfg(feature = "swagger")]
            {
                if !self.docs_cdn {
                    router = add_swagger_routes(router, spec_path, docs_path)?;
                } else {
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
            }
            #[cfg(not(feature = "swagger"))]
            {
                let _ = self.docs_cdn;
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
        }

        #[cfg(feature = "swagger")]
        if serve_openapi && let Some(swagger_path) = self.swagger_route {
            let spec_path = self.openapi_route.unwrap_or("/openapi.json");
            router = add_swagger_routes(router, spec_path, swagger_path)?;
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

        self.finish(router, schema_engine)
    }

    #[cfg(all(feature = "contract-runtime", not(feature = "contract")))]
    fn assemble_app(
        self,
        resolved: &[ResolvedRoute],
        schema_engine: Option<Arc<roundhouse_schema::engine::SchemaEngine>>,
    ) -> Result<Router<()>, BuildError> {
        let mut router = self.assemble(resolved, schema_engine.as_ref())?;

        for merged in &self.merges {
            router = merge_router(router, merged.router.clone(), merged.source)?;
        }

        if let Some(path) = self.openapi_route
            && let Some(bundle) = self.contract_bundle
        {
            let body = bundle.openapi;
            router = add_get_route(
                router,
                path,
                move || async move {
                    let mut response = axum::response::Response::new(axum::body::Body::from(body));
                    response.headers_mut().insert(
                        axum::http::header::CONTENT_TYPE,
                        axum::http::HeaderValue::from_static("application/json"),
                    );
                    response
                },
                crate::build_error::Source::new("App::openapi_route", "<app>", 0),
            )?;
        }

        if let Some(docs_path) = self.docs_route {
            let spec_path = self.openapi_route.unwrap_or("/openapi.json");
            #[cfg(feature = "swagger")]
            {
                if !self.docs_cdn {
                    router = add_swagger_routes(router, spec_path, docs_path)?;
                } else {
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
            }
            #[cfg(not(feature = "swagger"))]
            {
                let _ = self.docs_cdn;
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
        }

        #[cfg(feature = "swagger")]
        if let Some(swagger_path) = self.swagger_route {
            let spec_path = self.openapi_route.unwrap_or("/openapi.json");
            router = add_swagger_routes(router, spec_path, swagger_path)?;
        }

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

        self.finish(router, schema_engine)
    }

    #[cfg(not(feature = "contract-runtime"))]
    fn assemble_app(
        self,
        resolved: &[ResolvedRoute],
        schema_engine: Option<Arc<roundhouse_schema::engine::SchemaEngine>>,
    ) -> Result<Router<()>, BuildError> {
        let mut router = self.assemble(resolved, schema_engine.as_ref())?;

        for merged in &self.merges {
            router = merge_router(router, merged.router.clone(), merged.source)?;
        }

        // A served response is part of the contract: a method mismatch uses the same Problem
        // shape as the 404 fallback instead of axum's empty default body.
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

        self.finish(router, schema_engine)
    }

    fn finish(
        self,
        mut router: Router<()>,
        schema_engine: Option<Arc<roundhouse_schema::engine::SchemaEngine>>,
    ) -> Result<Router<()>, BuildError> {
        #[cfg(feature = "contract-runtime")]
        if let Some(bundle) = self.contract_bundle
            && (self.verify_responses || std::env::var_os("ROUNDHOUSE_VERIFY").is_some())
        {
            let validator = Arc::new(
                crate::ContractResponseValidator::from_contract_json(bundle.contract).map_err(
                    |error| BuildError::SchemaCompilation {
                        message: error.to_string(),
                    },
                )?,
            );
            router = router.layer(axum::middleware::from_fn_with_state(
                validator,
                crate::contract_response::validate_response,
            ));
        }

        if let Some(fallback) = self.fallback {
            router = fallback(router);
        }

        #[cfg(feature = "multipart")]
        let max_parts = self.max_parts;
        #[cfg(not(feature = "multipart"))]
        let max_parts = None;

        router = router.layer(axum::middleware::from_fn_with_state(
            ValidationDefaults {
                status: self.validation_status,
                mode: self.validation,
                max_body: self.max_body,
                max_parts,
                engine: schema_engine,
            },
            inject_validation_config,
        ));

        for layer in self.global_layers {
            router = layer(router);
        }

        Ok(router)
    }

    /// The ready `axum::Router<()>`.
    ///
    /// Application state has already been applied to every route, so the returned router needs no
    /// further `with_state` call. Embed it with `nest`, `merge` or `fallback_service`.
    pub fn into_router(self) -> Router<()> {
        self.build().router
    }

    /// Build and serve on `addr`, without a graceful-shutdown signal.
    ///
    /// Use [`App::serve_with`] to stop accepting connections and let in-flight requests finish
    /// when the supplied shutdown future resolves. This method is available with the `serve`
    /// feature, which is enabled by default.
    #[cfg(feature = "serve")]
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
    ///
    /// This method is available with the `serve` feature, which is enabled by default.
    #[cfg(feature = "serve")]
    pub async fn serve_with<F>(self, addr: impl Into<String>, shutdown: F) -> std::io::Result<()>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let addr: String = addr.into();
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        let built = self.build();
        tracing::info!("listening on http://{}", listener.local_addr()?);
        serve_listener(listener, built.router, shutdown).await
    }

    #[cfg(feature = "contract-runtime")]
    fn check_reserved_paths(
        &self,
        serve_openapi: bool,
        resolved: &[ResolvedRoute],
    ) -> Result<(), BuildError> {
        if !serve_openapi {
            return Ok(());
        }

        let mut reserved: Vec<(String, &'static str, crate::build_error::Source)> = Vec::new();
        if let Some(path) = self.openapi_route {
            reserved.push((
                prefix::join("", path),
                "the generated OpenAPI document",
                crate::build_error::Source::new("App::openapi_route", "<app>", 0),
            ));
        }
        #[cfg(feature = "contract-runtime")]
        {
            if let Some(path) = self.docs_route {
                let docs = prefix::join("", path);
                reserved.push((
                    docs.clone(),
                    "the documentation UI",
                    crate::build_error::Source::new("App::docs_route", "<app>", 0),
                ));
                #[cfg(feature = "swagger")]
                if !self.docs_cdn {
                    let asset_root = prefix::join(&docs, "assets");
                    for asset in super::assemble::SWAGGER_ASSETS {
                        reserved.push((
                            prefix::join(&asset_root, asset.name),
                            "a documentation UI asset",
                            crate::build_error::Source::new("App::docs_route", "<app>", 0),
                        ));
                    }
                }
            }
            #[cfg(feature = "swagger")]
            if let Some(path) = self.swagger_route {
                let swagger = prefix::join("", path);
                reserved.push((
                    swagger.clone(),
                    "the self-hosted Swagger UI",
                    crate::build_error::Source::new("App::swagger_route", "<app>", 0),
                ));
                let asset_root = prefix::join(&swagger, "assets");
                for asset in super::assemble::SWAGGER_ASSETS {
                    reserved.push((
                        prefix::join(&asset_root, asset.name),
                        "a self-hosted Swagger UI asset",
                        crate::build_error::Source::new("App::swagger_route", "<app>", 0),
                    ));
                }
            }
        }

        for (path, what, _) in &reserved {
            if let Some(route) = resolved.iter().find(|route| {
                prefix::join("", &route.path) == *path
                    && matches!(route.method, Method::Get | Method::Any)
            }) {
                return Err(BuildError::RouteConflict {
                    path: path.clone(),
                    reason: format!(
                        "{what} is served at this path; move the document route or the operation"
                    ),
                    source: route.record.source_ref(),
                });
            }
        }

        for (index, (path, what, source)) in reserved.iter().enumerate() {
            if let Some((_, other, _)) = reserved
                .iter()
                .skip(index + 1)
                .find(|(other_path, _, _)| other_path == path)
            {
                return Err(BuildError::RouteConflict {
                    path: path.clone(),
                    reason: format!("{what} and {other} cannot share a path"),
                    source: *source,
                });
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "contract-runtime"))]
    fn check_reserved_paths(
        &self,
        _serve_openapi: bool,
        _resolved: &[ResolvedRoute],
    ) -> Result<(), BuildError> {
        Ok(())
    }

    fn resolve(
        &self,
        diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    ) -> Result<Vec<ResolvedRoute>, BuildError> {
        #[cfg(feature = "contract")]
        let global_layer_docs = &self.global_layer_docs[..];
        #[cfg(not(feature = "contract"))]
        let global_layer_docs = &[][..];
        resolve_tree(
            &self.roots,
            self.strict,
            global_layer_docs,
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
    ) -> Result<Router<()>, BuildError> {
        let mut by_record: HashMap<*const RouteRecord, (String, MethodRouter<()>)> = HashMap::new();
        let mut absolute: Vec<(String, MethodRouter<()>)> = Vec::new();

        for route in resolved {
            let method_router =
                build_method_router::<S>(route, &self.state, engine, self.max_body)?;
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

/// Serve an already-bound listener with connection metadata installed.
///
/// `axum::serve(listener, router)` does not insert `ConnectInfo`; the
/// `into_make_service_with_connect_info` wrapper is required for handlers that take
/// `ConnectInfo<T>`.
#[cfg(feature = "serve")]
async fn serve_listener<F>(
    listener: tokio::net::TcpListener,
    router: Router<()>,
    shutdown: F,
) -> std::io::Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

#[cfg(all(test, feature = "serve"))]
mod tests {
    use super::*;
    use axum::extract::ConnectInfo;
    use axum::routing::get;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn peer(ConnectInfo(address): ConnectInfo<SocketAddr>) -> String {
        address.ip().to_string()
    }

    #[tokio::test]
    async fn serve_listener_exposes_connect_info() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = listener.local_addr().expect("listener address");
        let router = Router::new().route("/", get(peer));
        let server = tokio::spawn(serve_listener(
            listener,
            router,
            std::future::pending::<()>(),
        ));

        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect to test server");
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .await
            .expect("read response");

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("127.0.0.1"), "{response}");

        server.abort();
        let _ = server.await;
    }
}
