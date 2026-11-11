use std::any::TypeId;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use axum::Router;
use axum::routing::MethodRouter;

use crate::build_error::BuildError;
use crate::doc;
use crate::openapi::{Info, OpenApi};
use crate::prefix;
use crate::problem::Problem;
use crate::registry::{Method, Module, RouteRecord};

use super::app::App;
#[cfg(feature = "swagger")]
use super::assemble::add_swagger_routes;
use super::assemble::{
    add_get_route, build_method_router, build_module, docs_html, inject_validation_config,
    merge_router,
};
use super::built::{Built, ResolvedRoute};
use super::fragment::{DocumentRejections, MergeRecord};
use super::resolve::resolve_tree;

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
    pub fn try_build(self) -> Result<Built, BuildError> {
        let mut diagnostics = Vec::new();
        #[cfg(feature = "swagger")]
        let serve_swagger = self.swagger_route.is_some();
        #[cfg(not(feature = "swagger"))]
        let serve_swagger = false;
        let serve_openapi = self.info.is_some()
            || self.openapi_route.is_some()
            || self.docs_route.is_some()
            || serve_swagger;
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

    /// The ready `axum::Router<()>`.
    ///
    /// Application state has already been applied to every route, so the returned router needs no
    /// further `with_state` call. Embed it with `nest`, `merge` or `fallback_service`.
    pub fn into_router(self) -> Router<()> {
        self.build().router
    }

    /// The escape hatch: a real `axum::Router<S>`.
    ///
    /// State has already been provided inside; wrapping the ready router as a fallback service
    /// keeps the return type exactly what an axum user expects while preserving module layers and
    /// `404`/`405` semantics.
    #[deprecated(
        note = "use `App::into_router()`; state is already applied and the router needs no                 second `with_state` call"
    )]
    pub fn router(self) -> Router<S> {
        let built = self.build();
        Router::<S>::new().fallback_service(built.router)
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

        let mut reserved: Vec<(String, &'static str, crate::build_error::Source)> = Vec::new();
        if let Some(path) = self.openapi_route {
            reserved.push((
                prefix::join("", path),
                "the generated OpenAPI document",
                crate::build_error::Source::new("App::openapi_route", "<app>", 0),
            ));
        }
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
