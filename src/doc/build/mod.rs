//! Assemble the OpenAPI document from the same resolved routes the router uses.

use std::collections::BTreeMap;

use crate::auth::SchemeDoc;
use crate::build_error::{BuildError, Diagnostic};
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::{Info, OpenApi, Paths};

use crate::router::ResolvedRoute;

pub fn build(info: impl Into<Info>, routes: &[ResolvedRoute]) -> OpenApi {
    build_with_options(info, routes, true, false).unwrap_or_else(|error| panic!("{error}"))
}

pub fn build_with_options(
    info: impl Into<Info>,
    routes: &[ResolvedRoute],
    include_rejections: bool,
    strict: bool,
) -> Result<OpenApi, BuildError> {
    let mut diagnostics = Vec::new();
    let mut bodies = Vec::new();
    let mut parts = Vec::new();
    let mut route_bodies = std::collections::BTreeMap::new();
    build_with_observations(
        info,
        routes,
        include_rejections,
        strict,
        &mut diagnostics,
        &mut bodies,
        &mut parts,
        &mut route_bodies,
    )
}

/// Build the document, collecting non-fatal observations into `diagnostics`.
pub fn build_with_diagnostics(
    info: impl Into<Info>,
    routes: &[ResolvedRoute],
    include_rejections: bool,
    strict: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<OpenApi, BuildError> {
    let mut bodies = Vec::new();
    let mut parts = Vec::new();
    let mut route_bodies = std::collections::BTreeMap::new();
    build_with_observations(
        info,
        routes,
        include_rejections,
        strict,
        diagnostics,
        &mut bodies,
        &mut parts,
        &mut route_bodies,
    )
}

/// Build the document and collect every documented request body's inline schema, keyed by the
/// Rust type, so schema mode can enforce composite bodies (`Vec<T>`, `Option<T>`, maps, tuples).
#[allow(clippy::too_many_arguments)]
pub fn build_with_observations(
    info: impl Into<Info>,
    routes: &[ResolvedRoute],
    include_rejections: bool,
    strict: bool,
    diagnostics: &mut Vec<Diagnostic>,
    bodies: &mut Vec<(
        std::any::TypeId,
        utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
    )>,
    parts: &mut Vec<(
        std::any::TypeId,
        utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
    )>,
    route_bodies: &mut std::collections::BTreeMap<String, (std::any::TypeId, String)>,
) -> Result<OpenApi, BuildError> {
    let _registration = crate::doc::registration::Guard::new();
    let info = info.into();
    let mut components = utoipa::openapi::schema::Components::new();
    let mut paths = Paths::new();
    let mut used_schemes: BTreeMap<String, (SchemeDoc, crate::build_error::Source)> =
        BTreeMap::new();

    let mut has_hidden = false;
    for route in routes {
        if route.hidden || route.record.hidden {
            has_hidden = true;
            continue;
        }
        let Some(http_method) = route.method.to_utoipa() else {
            // `any` hides the method set; it is served but not documented.
            crate::build_error::report(
                diagnostics,
                false,
                Diagnostic::new(
                    route.record.source_ref(),
                    format!(
                        "`any` route `{}` is not documented; declare explicit methods to include it",
                        route.path
                    ),
                ),
            )?;
            continue;
        };

        crate::doc::registration::set_source(route.record.source_ref());
        let mut docs = (route.record.docs)(
            &mut components,
            route.record.validate && route.record.validate_body,
        );
        if let Some(custom) = route.record.doc {
            custom(&mut docs);
        }
        if !docs.unbounded_file_fields.is_empty() {
            crate::build_error::report(
                diagnostics,
                strict,
                Diagnostic::new(
                    route.record.source_ref(),
                    format!(
                        "operation `{}` declares file field(s) `{}` without `max_bytes`; they are \
                         bounded only by the app-wide body limit",
                        route.operation_id,
                        docs.unbounded_file_fields.join("`, `")
                    ),
                ),
            )?;
        }
        // `validate_body = false` is a precise, intentional body opt-out; `unverified = "..."`
        // acknowledges the remaining markers on the whole operation. Neither should make a
        // strict build fail or print a warning: the document already records the opt-out.
        let acknowledged = route.record.unverified.is_some();
        if docs.unverified && route.record.validate_body && !acknowledged {
            if strict {
                return Err(BuildError::UnverifiedBody {
                    operation_id: route.operation_id.clone(),
                    source: route.record.source_ref(),
                });
            }
            diagnostics.push(Diagnostic::new(
                route.record.source_ref(),
                format!(
                    "operation `{}` takes a body extractor the document cannot verify; it is \
                     marked `x-roundhouse-unverified-body`",
                    route.operation_id
                ),
            ));
        }
        if docs.unverified_response && !acknowledged {
            if strict {
                return Err(BuildError::UnverifiedResponse {
                    operation_id: route.operation_id.clone(),
                    source: route.record.source_ref(),
                });
            }
            diagnostics.push(Diagnostic::new(
                route.record.source_ref(),
                format!(
                    "operation `{}` returns a response type the document cannot verify; it is \
                     marked `x-roundhouse-unverified-response`",
                    route.operation_id
                ),
            ));
        }
        if let Some(error_type) = docs.undocumented_error {
            crate::build_error::report(
                diagnostics,
                strict,
                Diagnostic::new(
                    route.record.source_ref(),
                    format!(
                        "operation `{}` returns `{error_type}` but that type registers no \
                         documented responses; derive `ApiError`/`ApiResponse` or return `Problem`",
                        route.operation_id
                    ),
                ),
            )?;
        }
        let operation = operation(
            route,
            docs,
            &mut components,
            &mut used_schemes,
            include_rejections,
            strict,
            diagnostics,
            bodies,
            parts,
            route_bodies,
        )?;
        paths.add_path_operation(&route.path, vec![http_method], operation);
    }

    if let Some(collision) = crate::doc::registration::finish().into_iter().next() {
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

mod operation;
mod pieces;

use operation::operation;
