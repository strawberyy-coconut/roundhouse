use std::collections::BTreeMap;

use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::path::OperationBuilder;
use utoipa::openapi::{Deprecated, ResponsesBuilder, SecurityRequirement};

use super::super::{HandlerDocs, Param, ParamIn, ResponseDoc};
use super::pieces::{
    build_body, build_param, build_response, merge_response, merge_scheme, requirement,
};
use crate::auth::SchemeDoc;
use crate::build_error::{BuildError, Diagnostic};
use crate::router::ResolvedRoute;

#[allow(clippy::too_many_arguments)]
pub(super) fn operation(
    route: &ResolvedRoute,
    docs: HandlerDocs,
    components: &mut utoipa::openapi::schema::Components,
    used_schemes: &mut BTreeMap<String, (SchemeDoc, crate::build_error::Source)>,
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
) -> Result<utoipa::openapi::path::Operation, BuildError> {
    let unverified = docs.unverified;
    let unverified_response = docs.unverified_response;
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
            // with one capture. With several captures (a module prefix plus a leaf, say) a string is the
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
                super::super::schema::string()
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
    // A module guard plus `Option<Auth<A>>` on the handler must not publish the same scheme as
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
    for part in &docs.parts {
        parts.push(((part.type_id)(), (part.schema)(components)));
    }
    if let Some(body) = docs.body {
        if let Some(body_type) = docs.body_type {
            let type_id = body_type();
            route_bodies.insert(
                route.operation_id.clone(),
                (type_id, body.content_type.clone()),
            );
            bodies.push((type_id, body.schema.clone()));
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
    if unverified_response {
        extensions = extensions.add("x-roundhouse-unverified-response", true);
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
