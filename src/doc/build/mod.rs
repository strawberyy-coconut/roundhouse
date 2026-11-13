//! Assemble the OpenAPI document from the same resolved routes the router uses.

use std::any::TypeId;
use std::collections::BTreeMap;

use crate::auth::SchemeDoc;
use crate::build_error::{BuildError, Diagnostic};
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::{Info, OpenApi, Paths};
use utoipa::openapi::{RefOr, schema::Schema};

use crate::router::ResolvedRoute;

type InlineSchema = (TypeId, RefOr<Schema>);
type RouteBody = (TypeId, String);

/// Side channels collected while the document is built.
///
/// The public wrappers hide this; the app builder consumes the schemas for validation and keeps
/// the diagnostics for `Built`.
#[derive(Default)]
pub(crate) struct Observations {
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) bodies: Vec<InlineSchema>,
    pub(crate) parts: Vec<InlineSchema>,
    pub(crate) route_bodies: BTreeMap<String, RouteBody>,
}

pub fn build(info: impl Into<Info>, routes: &[ResolvedRoute]) -> OpenApi {
    build_with_options(info, routes, true, false).unwrap_or_else(|error| panic!("{error}"))
}

pub fn build_with_options(
    info: impl Into<Info>,
    routes: &[ResolvedRoute],
    include_rejections: bool,
    strict: bool,
) -> Result<OpenApi, BuildError> {
    let mut observations = Observations::default();
    build_with_observations(info, routes, include_rejections, strict, &mut observations)
}

/// Build the document, collecting non-fatal observations into `diagnostics`.
pub fn build_with_diagnostics(
    info: impl Into<Info>,
    routes: &[ResolvedRoute],
    include_rejections: bool,
    strict: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<OpenApi, BuildError> {
    let mut observations = Observations::default();
    let result =
        build_with_observations(info, routes, include_rejections, strict, &mut observations);
    diagnostics.append(&mut observations.diagnostics);
    result
}

/// Build the document and collect every documented request body's inline schema, keyed by the
/// Rust type, so schema mode can enforce composite bodies (`Vec<T>`, `Option<T>`, maps, tuples).
pub(crate) fn build_with_observations(
    info: impl Into<Info>,
    routes: &[ResolvedRoute],
    include_rejections: bool,
    strict: bool,
    observations: &mut Observations,
) -> Result<OpenApi, BuildError> {
    DocumentBuild {
        routes,
        include_rejections,
        strict,
        diagnostics: &mut observations.diagnostics,
        bodies: &mut observations.bodies,
        parts: &mut observations.parts,
        route_bodies: &mut observations.route_bodies,
    }
    .run(info)
}

/// One document-build invocation and the observations it accumulates.
pub(super) struct DocumentBuild<'a> {
    routes: &'a [ResolvedRoute],
    include_rejections: bool,
    strict: bool,
    diagnostics: &'a mut Vec<Diagnostic>,
    bodies: &'a mut Vec<InlineSchema>,
    parts: &'a mut Vec<InlineSchema>,
    route_bodies: &'a mut BTreeMap<String, RouteBody>,
}

impl DocumentBuild<'_> {
    pub(super) fn run(mut self, info: impl Into<Info>) -> Result<OpenApi, BuildError> {
        let info = info.into();
        let mut components = utoipa::openapi::schema::Components::new();
        let mut context = crate::doc::DocContext::new(&mut components);
        let mut paths = Paths::new();
        let mut used_schemes: BTreeMap<String, (SchemeDoc, crate::build_error::Source)> =
            BTreeMap::new();

        let mut has_hidden = false;
        for route in self.routes {
            if route.hidden || route.record.hidden {
                has_hidden = true;
                continue;
            }
            let Some(http_method) = route.method.to_utoipa() else {
                // `any` hides the method set; it is served but not documented.
                crate::build_error::report(
                    self.diagnostics,
                    false,
                    Diagnostic::new(
                        route.record.source_ref(),
                        format!(
                            "`any` route `{}` is not documented; declare explicit methods to \
                             include it",
                            route.path
                        ),
                    ),
                )?;
                continue;
            };

            context.set_source(route.record.source_ref());
            let mut docs = (route.record.docs)(&mut context);
            if route.record.validation.validates_body() && docs.verification.body_verified() {
                docs.verification.clear_body();
            }
            if let Some(custom) = route.record.doc {
                custom(&mut docs);
            }
            if !docs.verification.unbounded_file_fields().is_empty() {
                crate::build_error::report(
                    self.diagnostics,
                    self.strict,
                    Diagnostic::new(
                        route.record.source_ref(),
                        format!(
                            "operation `{}` declares file field(s) `{}` without `max_bytes`; they \
                             are bounded only by the app-wide body limit",
                            route.operation_id,
                            docs.verification.unbounded_file_fields().join("`, `")
                        ),
                    ),
                )?;
            }
            // `validate_body = false` is a precise, intentional body opt-out; `unverified = "..."`
            // acknowledges the remaining markers on the whole operation. Neither should make a
            // strict build fail or print a warning: the document already records the opt-out.
            let acknowledged = route.record.unverified.is_some();
            if docs.verification.body()
                && !route.record.validation.body_opt_out_is_explicit()
                && !acknowledged
            {
                if self.strict {
                    return Err(BuildError::UnverifiedBody {
                        operation_id: route.operation_id.clone(),
                        source: route.record.source_ref(),
                    });
                }
                self.diagnostics.push(Diagnostic::new(
                    route.record.source_ref(),
                    format!(
                        "operation `{}` takes a body extractor the document cannot verify; it is \
                         marked `x-roundhouse-unverified-body`",
                        route.operation_id
                    ),
                ));
            }
            if docs.verification.response() && !acknowledged {
                if self.strict {
                    return Err(BuildError::UnverifiedResponse {
                        operation_id: route.operation_id.clone(),
                        source: route.record.source_ref(),
                    });
                }
                self.diagnostics.push(Diagnostic::new(
                    route.record.source_ref(),
                    format!(
                        "operation `{}` returns a response type the document cannot verify; it is \
                         marked `x-roundhouse-unverified-response`",
                        route.operation_id
                    ),
                ));
            }
            if let Some(error_type) = docs.verification.undocumented_error() {
                crate::build_error::report(
                    self.diagnostics,
                    self.strict,
                    Diagnostic::new(
                        route.record.source_ref(),
                        format!(
                            "operation `{}` returns `{error_type}` but that type registers no \
                             documented responses; derive `ApiError`/`ApiResponse` or return \
                             `Problem`",
                            route.operation_id
                        ),
                    ),
                )?;
            }
            let operation = self.operation(route, docs, &mut context, &mut used_schemes)?;
            paths.add_path_operation(&route.path, vec![http_method], operation);
        }

        let collisions = context.take_collisions();
        drop(context);
        if let Some(collision) = collisions.into_iter().next() {
            return Err(BuildError::ComponentCollision {
                name: collision.name,
                first_type: collision.first_type,
                second_type: collision.second_type,
                source: collision.source,
            });
        }

        for (scheme, _) in used_schemes.values() {
            components
                .security_schemes
                .insert(scheme.name.to_string(), scheme.kind.to_utoipa().into());
        }

        let mut document = OpenApi::new(info, paths);
        if has_hidden {
            document.extensions = Some(
                ExtensionsBuilder::new()
                    .add("x-roundhouse-hidden", true)
                    .build(),
            );
        }
        if !components.schemas.is_empty()
            || !components.security_schemes.is_empty()
            || !components.responses.is_empty()
        {
            document.components = Some(components);
        }
        super::validate_document(&document)
            .map_err(|message| BuildError::InvalidDocument { message })?;
        Ok(document)
    }
}

mod operation;
mod pieces;
