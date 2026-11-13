use std::collections::BTreeMap;

use utoipa::openapi::extensions::ExtensionsBuilder;
use utoipa::openapi::path::OperationBuilder;
use utoipa::openapi::{Deprecated, ResponsesBuilder, SecurityRequirement};

use super::super::{OperationDoc, Param, ParamIn, ResponseDoc};
use super::pieces::{
    build_body, build_param, build_response, merge_response, merge_scheme, requirement,
};
use crate::auth::SchemeDoc;
use crate::build_error::{BuildError, Diagnostic};
use crate::router::ResolvedRoute;

use super::DocumentBuild;

impl DocumentBuild<'_> {
    pub(super) fn operation(
        &mut self,
        route: &ResolvedRoute,
        mut docs: OperationDoc,
        context: &mut crate::doc::DocContext<'_>,
        used_schemes: &mut BTreeMap<String, (SchemeDoc, crate::build_error::Source)>,
    ) -> Result<utoipa::openapi::path::Operation, BuildError> {
        let unverified = docs.verification.body();
        let unverified_response = docs.verification.response();
        let unverified_extractor = docs.verification.extractor();
        self.merge_operation_contract(route, &mut docs, unverified_extractor)?;
        let parameters = self.resolve_operation_parameters(route, &mut docs)?;
        let (responses, overridden) = self.resolve_operation_responses(route, &mut docs)?;
        let security = self.resolve_operation_security(route, &mut docs, used_schemes)?;

        let mut tags: Vec<String> = Vec::new();
        for tag in route.tags.iter().chain(route.record.tags.iter()) {
            if !tags.iter().any(|existing| existing == tag) {
                tags.push((*tag).to_string());
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
            .summary(docs.summary.clone())
            .description(docs.description.clone())
            .responses(responses);

        if !tags.is_empty() {
            builder = builder.tags(Some(tags));
        }
        for part in &docs.parts {
            self.parts.push(((part.type_id)(), (part.schema)(context)));
        }
        if let Some(body) = docs.body {
            if route.record.validation.validates_body()
                && let Some(body_type) = docs.body_type
            {
                let type_id = body_type();
                self.route_bodies.insert(
                    route.operation_id.clone(),
                    (type_id, body.content_type.clone()),
                );
                self.bodies.push((type_id, body.schema.clone()));
            }
            builder = builder.request_body(Some(build_body(body)));
        }
        if !parameters.is_empty() {
            builder = builder.parameters(Some(parameters.into_iter().map(build_param)));
        }
        if route.auth_cleared && security.is_empty() {
            // An explicit `anonymous` route overrides a document-level requirement. OpenAPI uses
            // an empty security array for "no security".
            builder = builder.securities(Some(Vec::new()));
        } else if !security.is_empty() {
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
        match route.record.validation {
            crate::registry::ValidationPolicy::Off => {
                extensions = extensions.add("x-roundhouse-validation", false);
            }
            crate::registry::ValidationPolicy::PartsOnly => {
                extensions = extensions.add("x-roundhouse-body-validation", false);
            }
            crate::registry::ValidationPolicy::Schema => {}
        }
        if !self.include_rejections {
            extensions = extensions.add("x-roundhouse-rejections", "none");
        }
        let extensions = extensions.build();
        if !extensions.is_empty() {
            builder = builder.extensions(Some(extensions));
        }

        Ok(builder.build())
    }

    /// Merge documentation supplied by layers and route guards, and report an extractor the
    /// macro could not infer.
    fn merge_operation_contract(
        &mut self,
        route: &ResolvedRoute,
        docs: &mut OperationDoc,
        unverified_extractor: bool,
    ) -> Result<(), BuildError> {
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

        if unverified_extractor && route.record.unverified.is_none() {
            crate::build_error::report(
                self.diagnostics,
                self.strict,
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
        Ok(())
    }

    /// Deduplicate parameters, apply `@path:` prose and fill every template capture.
    fn resolve_operation_parameters(
        &mut self,
        route: &ResolvedRoute,
        docs: &mut OperationDoc,
    ) -> Result<Vec<Param>, BuildError> {
        let path_descriptions = std::mem::take(&mut docs.path_descriptions);
        let mut parameters: Vec<Param> = Vec::new();
        for param in std::mem::take(&mut docs.params) {
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

        // A handler `@path:` description fills a scalar `Path<T>` parameter that has no field-level
        // description. A `PathParams` field description stays authoritative.
        for (name, description) in &path_descriptions {
            if let Some(param) = parameters
                .iter_mut()
                .find(|param| param.location == ParamIn::Path && &param.name == name)
                && param.description.is_none()
            {
                param.description = Some(description.clone());
            }
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
                        self.diagnostics,
                        self.strict,
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
            let mut param = Param::new(name.clone(), ParamIn::Path, schema);
            if let Some((_, description)) = path_descriptions
                .iter()
                .find(|(param_name, _)| param_name == name)
            {
                param.description = Some(description.clone());
            }
            parameters.push(param);
        }
        // A handler `@path:` name that is not a template capture is a declaration error.
        for (name, _) in &path_descriptions {
            if !captures.contains(name) {
                return Err(BuildError::UnknownPathParameter {
                    name: name.clone(),
                    path: route.path.clone(),
                    source: route.record.source_ref(),
                });
            }
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
        Ok(parameters)
    }

    /// Merge inferred, rejection and `@responses:` declarations into one response table.
    fn resolve_operation_responses(
        &mut self,
        route: &ResolvedRoute,
        docs: &mut OperationDoc,
    ) -> Result<(BTreeMap<u16, ResponseDoc>, bool), BuildError> {
        let mut responses: BTreeMap<u16, ResponseDoc> = BTreeMap::new();
        let failures = if self.include_rejections {
            std::mem::take(&mut docs.failures)
        } else {
            Vec::new()
        };
        for response in std::mem::take(&mut docs.responses)
            .into_iter()
            .chain(failures)
        {
            merge_response(
                &mut responses,
                response,
                route,
                self.strict,
                self.diagnostics,
            )?;
        }
        // The handler's `@responses:` declarations may add statuses or, with
        // `@responses(override):`, replace an inferred body. Replacing it silently would let the
        // documentation contradict the handler.
        let declared_responses = std::mem::take(&mut docs.declared_responses);
        let declared_override = docs.declared_response_override;
        let mut overridden = false;
        for response in declared_responses {
            // A bare status (`@responses: 200`) declares the status as a success response. When the
            // handler already inferred a response for that status, keep the inferred shape: the
            // declaration is an assertion, not a replacement. This is what lets a `Result` alias
            // keep the compile-time success guarantee without repeating the body type.
            let bare = response.schema.is_none() && response.content_type.is_none();
            if bare && responses.contains_key(&response.status) {
                continue;
            }
            let same_shape = responses.get(&response.status).is_some_and(|existing| {
                existing.schema == response.schema && existing.content_type == response.content_type
            });
            if same_shape {
                if let Some(existing) = responses.get_mut(&response.status) {
                    // A declaration without a richer description must not erase one supplied by a
                    // type/variant or a `doc` hook.
                    let canonical = crate::doc::canonical_reason(existing.status);
                    if existing.description == canonical
                        && response.description != crate::doc::canonical_reason(response.status)
                    {
                        existing.description = response.description;
                    }
                    for header in response.headers {
                        if !existing
                            .headers
                            .iter()
                            .any(|existing| existing.name == header.name)
                        {
                            existing.headers.push(header);
                        }
                    }
                }
                continue;
            }
            if responses.contains_key(&response.status) && !declared_override {
                return Err(BuildError::ConflictingResponse {
                    operation_id: route.operation_id.clone(),
                    status: response.status,
                    source: route.record.source_ref(),
                });
            }
            if responses.contains_key(&response.status) {
                overridden = true;
            }
            responses.insert(response.status, response);
        }

        for (status, description) in std::mem::take(&mut docs.response_descriptions) {
            if let Some(response) = responses.get_mut(&status) {
                response.description = description;
            } else {
                let label = if status == 0 {
                    "default".to_string()
                } else {
                    status.to_string()
                };
                crate::build_error::report(
                    self.diagnostics,
                    self.strict,
                    Diagnostic::new(
                        route.record.source_ref(),
                        format!(
                            "operation `{}` describes response {label}, but no such response is \
                         documented",
                            route.operation_id
                        ),
                    ),
                )?;
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
        Ok((responses, overridden))
    }

    /// Build the operation's security alternatives and register every used scheme.
    fn resolve_operation_security(
        &mut self,
        route: &ResolvedRoute,
        docs: &mut OperationDoc,
        used_schemes: &mut BTreeMap<String, (SchemeDoc, crate::build_error::Source)>,
    ) -> Result<Vec<SecurityRequirement>, BuildError> {
        for scheme in &docs.schemes {
            merge_scheme(used_schemes, *scheme, route.record.source_ref())?;
        }

        let required = std::mem::take(&mut docs.required_auth);
        // A module guard plus `Option<Auth<A>>` on the handler must not publish the same scheme as
        // both required and optional.
        let optional: Vec<_> = std::mem::take(&mut docs.optional_auth)
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
        Ok(security)
    }
}
