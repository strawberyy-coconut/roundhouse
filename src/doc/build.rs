//! Assemble the OpenAPI document from the same resolved routes the router uses.

use std::collections::BTreeMap;

use crate::auth::SchemeDoc;
use crate::build_error::{BuildError, Diagnostic};
use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::path::{OperationBuilder, ParameterBuilder};
use utoipa::openapi::request_body::{RequestBody, RequestBodyBuilder};
use utoipa::openapi::{
    Content, Deprecated, Info, OpenApi, Paths, Required, Response, ResponseBuilder,
    ResponsesBuilder, SecurityRequirement,
};

use super::{BodyDoc, Param, ParamIn, ResponseDoc};
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
    build_with_observations(
        info,
        routes,
        include_rejections,
        strict,
        &mut diagnostics,
        &mut bodies,
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
    build_with_observations(
        info,
        routes,
        include_rejections,
        strict,
        diagnostics,
        &mut bodies,
    )
}

/// Build the document and collect every documented request body's inline schema, keyed by the
/// Rust type, so schema mode can enforce composite bodies (`Vec<T>`, `Option<T>`, maps, tuples).
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
) -> Result<OpenApi, BuildError> {
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

        let mut docs = (route.record.docs)(&mut components);
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
                        "operation `{}` declares file field(s) `{}` without `max_bytes`; they are                          bounded only by the app-wide body limit",
                        route.operation_id,
                        docs.unbounded_file_fields.join("`, `")
                    ),
                ),
            )?;
        }
        if docs.unverified {
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
        )?;
        paths.add_path_operation(&route.path, vec![http_method], operation);
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

#[allow(clippy::too_many_arguments)]
fn operation(
    route: &ResolvedRoute,
    docs: super::HandlerDocs,
    components: &mut utoipa::openapi::schema::Components,
    used_schemes: &mut BTreeMap<String, (SchemeDoc, crate::build_error::Source)>,
    include_rejections: bool,
    strict: bool,
    diagnostics: &mut Vec<Diagnostic>,
    bodies: &mut Vec<(
        std::any::TypeId,
        utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
    )>,
) -> Result<utoipa::openapi::path::Operation, BuildError> {
    let unverified = docs.unverified;
    let unverified_extractor = docs.unverified_extractor;
    let mut docs = docs;
    for layer in &route.layer_docs {
        docs.params.extend(layer.parameters.clone());
        if docs.body.is_none() {
            docs.body = layer.body.clone();
        }
        docs.responses.extend(layer.responses.clone());
        docs.required_auth.extend(layer.required_auth.clone());
        docs.optional_auth.extend(layer.optional_auth.clone());
        docs.schemes.extend(layer.schemes.clone());
    }

    // Mount- and entry-level guards are installed on the method router by construction, so
    // their requirements are enforced, not asserted. A handler's scoped `Auth<A, Scopes>`
    // declaration wins over the bare guard for the same scheme.
    for guard in &route.auth {
        if !docs
            .required_auth
            .iter()
            .any(|auth| auth.name == guard.name)
        {
            docs.required_auth.push(guard.doc());
        }
        if !docs.schemes.iter().any(|scheme| scheme.name == guard.name) {
            docs.schemes.push(guard.scheme);
        }
        docs.failures.extend((guard.failures)());
    }
    // A guard turns an `Option<Auth<A, Scopes>>` alternative into a requirement. Keep the
    // handler's scopes on the promoted requirement so the document still names them.
    for optional in &docs.optional_auth {
        if let Some(required) = docs
            .required_auth
            .iter_mut()
            .find(|required| required.name == optional.name)
            && required.scopes.is_empty()
        {
            required.scopes = optional.scopes;
        }
    }

    if unverified_extractor {
        crate::build_error::report(
            diagnostics,
            strict,
            Diagnostic::new(
                route.record.source_ref(),
                format!(
                    "operation `{}` takes an extractor whose parameters the document cannot \
                     infer; it is marked `x-roundhouse-unverified-extractor`",
                    route.operation_id
                ),
            ),
        )?;
    }

    let mut parameters: Vec<Param> = Vec::new();
    for param in docs.params {
        if let Some(existing) = parameters
            .iter()
            .find(|existing| existing.name == param.name && existing.location == param.location)
        {
            if existing.schema != param.schema {
                return Err(BuildError::ConflictingParameter {
                    name: param.name.clone(),
                    location: param.location,
                    source: route.record.source_ref(),
                });
            }
            continue;
        }
        parameters.push(param);
    }

    // OpenAPI 3.1 requires a parameter object for every `{capture}` in the template. A scalar or
    // foreign `Path<T>` cannot supply a name, so fill in the template's own name with the scalar's
    // schema (a string when even that is unknown) instead of emitting an invalid document.
    let captures = crate::prefix::captures(&route.path);
    for name in &captures {
        if parameters
            .iter()
            .any(|param| param.location == ParamIn::Path && &param.name == name)
        {
            continue;
        }
        let schema = if captures.len() == 1 {
            // A scalar `Path<T>` carries one schema and no name; it can only describe a template
            // with one capture. With several captures (a mount plus a leaf, say) a string is the
            // honest fallback, and the diagnostic below asks for a typed extractor.
            docs.path_schema.and_then(|schema| schema())
        } else {
            None
        };
        let schema = match schema {
            Some(schema) => schema,
            None => {
                crate::build_error::report(
                    diagnostics,
                    strict,
                    Diagnostic::new(
                        route.record.source_ref(),
                        format!(
                            "path capture `{name}` in `{}` is documented as a string because no \
                             `Path<T>` covers it; add a Path extractor or declare its type",
                            route.path
                        ),
                    ),
                )?;
                super::schema::string()
            }
        };
        parameters.push(Param::new(name.clone(), ParamIn::Path, schema));
    }
    // ... and a path parameter that no capture uses is equally invalid.
    for param in &parameters {
        if param.location == ParamIn::Path && !captures.contains(&param.name) {
            return Err(BuildError::UnknownPathParameter {
                name: param.name.clone(),
                path: route.path.clone(),
                source: route.record.source_ref(),
            });
        }
    }

    let mut responses: BTreeMap<u16, ResponseDoc> = BTreeMap::new();
    let failures = if include_rejections {
        docs.failures
    } else {
        Vec::new()
    };
    for response in docs.responses.into_iter().chain(failures) {
        merge_response(&mut responses, response, route, strict, diagnostics)?;
    }
    // A route-level `responses(..)` declaration may add statuses or, with `override`, replace an
    // inferred body. Replacing it silently would let the table contradict the handler.
    let mut overridden = false;
    for response in (route.record.responses)(components) {
        match responses.get(&response.status) {
            Some(existing)
                if existing.schema != response.schema
                    || existing.content_type != response.content_type =>
            {
                if !route.record.response_override {
                    return Err(BuildError::ConflictingResponse {
                        operation_id: route.operation_id.clone(),
                        status: response.status,
                        source: route.record.source_ref(),
                    });
                }
                overridden = true;
                responses.insert(response.status, response);
            }
            _ => {
                responses.insert(response.status, response);
            }
        }
    }

    if !responses
        .keys()
        .any(|status| *status == 0 || (200..400).contains(status))
    {
        return Err(BuildError::UndeclaredSuccess {
            operation_id: route.operation_id.clone(),
            source: route.record.source_ref(),
        });
    }

    let mut tags: Vec<String> = Vec::new();
    for tag in route.tags.iter().chain(route.record.tags.iter()) {
        if !tags.iter().any(|existing| existing == tag) {
            tags.push((*tag).to_string());
        }
    }

    for scheme in &docs.schemes {
        merge_scheme(used_schemes, *scheme, route.record.source_ref())?;
    }

    let required = docs.required_auth;
    // A mount guard plus `Option<Auth<A>>` on the handler must not publish the same scheme as
    // both required and optional.
    let optional: Vec<_> = docs
        .optional_auth
        .into_iter()
        .filter(|optional| !required.iter().any(|auth| auth.name == optional.name))
        .collect();

    let mut security: Vec<SecurityRequirement> = Vec::new();
    for auth in required.iter().chain(optional.iter()) {
        let Some(scheme) = docs
            .schemes
            .iter()
            .find(|scheme| scheme.name == auth.name)
            .copied()
        else {
            // No `Auth<A>` or `layer_documented(auth::<A>())` registered the kind.
            return Err(BuildError::UnverifiedSecurity {
                name: auth.name.to_string(),
                source: route.record.source_ref(),
            });
        };
        merge_scheme(used_schemes, scheme, route.record.source_ref())?;
    }

    if !required.is_empty() || !optional.is_empty() {
        if optional.is_empty() {
            security.push(requirement(&required));
        } else {
            let mut all = required.clone();
            all.extend(optional.iter().copied());
            security.push(requirement(&all));
            if required.is_empty() {
                security.push(SecurityRequirement::default());
            } else {
                security.push(requirement(&required));
            }
        }
    }

    let responses = responses
        .into_values()
        .fold(ResponsesBuilder::new(), |builder, response| {
            let key = if response.status == 0 {
                "default".to_string()
            } else {
                response.status.to_string()
            };
            builder.response(key, build_response(response))
        })
        .build();

    let mut builder = OperationBuilder::new()
        .operation_id(Some(route.operation_id.clone()))
        .summary(route.record.summary)
        .description(route.record.description)
        .responses(responses);

    if !tags.is_empty() {
        builder = builder.tags(Some(tags));
    }
    if let Some(body) = docs.body {
        if let Some(body_type) = docs.body_type {
            bodies.push((body_type(), body.schema.clone()));
        }
        builder = builder.request_body(Some(build_body(body)));
    }
    if !parameters.is_empty() {
        builder = builder.parameters(Some(parameters.into_iter().map(build_param)));
    }
    if !security.is_empty() {
        builder = builder.securities(Some(security));
    }
    if route.record.deprecated {
        builder = builder.deprecated(Some(Deprecated::True));
    }

    let mut extensions = ExtensionsBuilder::new();
    if unverified {
        extensions = extensions.add("x-roundhouse-unverified-body", true);
    }
    if unverified_extractor {
        extensions = extensions.add("x-roundhouse-unverified-extractor", true);
    }
    if overridden {
        extensions = extensions.add("x-roundhouse-overridden-response", true);
    }
    if route.record.public {
        extensions = extensions.add("x-roundhouse-public", true);
    }
    if !route.record.validate {
        extensions = extensions.add("x-roundhouse-validation", false);
    }
    if !include_rejections {
        extensions = extensions.add("x-roundhouse-rejections", "none");
    }
    let extensions = extensions.build();
    if !extensions.is_empty() {
        builder = builder.extensions(Some(extensions));
    }

    Ok(builder.build())
}

fn build_param(param: Param) -> utoipa::openapi::path::Parameter {
    let parameter_in = match param.location {
        ParamIn::Path => utoipa::openapi::path::ParameterIn::Path,
        ParamIn::Query => utoipa::openapi::path::ParameterIn::Query,
        ParamIn::Header => utoipa::openapi::path::ParameterIn::Header,
        ParamIn::Cookie => utoipa::openapi::path::ParameterIn::Cookie,
    };
    ParameterBuilder::new()
        .name(param.name)
        .parameter_in(parameter_in)
        .required(if param.required {
            Required::True
        } else {
            Required::False
        })
        .schema(Some(param.schema))
        .description(param.description)
        .style(param.style)
        .explode(param.explode)
        .build()
}

fn build_body(body: BodyDoc) -> RequestBody {
    let mut content = Content::new(Some(body.schema));
    content.encoding = body.encoding;
    let mut builder = RequestBodyBuilder::new()
        .required(Some(if body.required {
            Required::True
        } else {
            Required::False
        }))
        .content(body.content_type, content);
    if let Some(description) = body.description {
        builder = builder.description(Some(description));
    }
    builder.build()
}

fn build_response(response: ResponseDoc) -> Response {
    let mut builder = ResponseBuilder::new().description(response.description);
    if let (Some(content_type), Some(schema)) = (response.content_type, response.schema) {
        builder = builder.content(content_type, Content::new(Some(schema)));
    }
    for header in response.headers {
        builder = builder.header(
            header.name,
            utoipa::openapi::HeaderBuilder::new()
                .schema(Some(header.schema))
                .build(),
        );
    }
    builder.build()
}

fn merge_response(
    responses: &mut BTreeMap<u16, ResponseDoc>,
    response: ResponseDoc,
    route: &ResolvedRoute,
    strict: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), BuildError> {
    match responses.get_mut(&response.status) {
        None => {
            responses.insert(response.status, response);
        }
        Some(existing) => {
            if existing.schema == response.schema && existing.content_type == response.content_type
            {
                for header in response.headers {
                    if !existing
                        .headers
                        .iter()
                        .any(|existing| existing.name == header.name)
                    {
                        existing.headers.push(header);
                    }
                }
                return Ok(());
            }
            if response.status >= 400 {
                // Two shapes for one status is a lie; collapse to Problem, which is what every
                // extractor and ApiError produce.
                responses.insert(
                    response.status,
                    ResponseDoc::problem(response.status, super::canonical_reason(response.status)),
                );
            } else {
                crate::build_error::report(
                    diagnostics,
                    strict,
                    Diagnostic::new(
                        route.record.source_ref(),
                        format!(
                            "two different success bodies share status {}; keeping the first",
                            response.status
                        ),
                    ),
                )?;
            }
        }
    }
    Ok(())
}

fn requirement(auth: &[crate::auth::AuthDoc]) -> SecurityRequirement {
    let mut requirement = SecurityRequirement::default();
    for doc in auth {
        requirement = requirement.add(doc.name, doc.scopes.iter().copied());
    }
    requirement
}

/// Store the first declaration of a scheme; a second, different kind is a build error that
/// points at both declarations.
fn merge_scheme(
    used: &mut BTreeMap<String, (SchemeDoc, crate::build_error::Source)>,
    scheme: SchemeDoc,
    source: crate::build_error::Source,
) -> Result<(), BuildError> {
    match used.get(scheme.name) {
        Some((existing, first)) if existing.kind != scheme.kind => {
            return Err(BuildError::SchemeKindConflict {
                name: scheme.name.to_string(),
                first: *first,
                second: source,
            });
        }
        Some(_) => {}
        None => {
            used.insert(scheme.name.to_string(), (scheme, source));
        }
    }
    Ok(())
}
