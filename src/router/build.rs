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
