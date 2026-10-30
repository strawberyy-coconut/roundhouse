use std::collections::BTreeMap;

use utoipa::openapi::path::ParameterBuilder;
use utoipa::openapi::request_body::{RequestBody, RequestBodyBuilder};
use utoipa::openapi::{Content, Required, Response, ResponseBuilder, SecurityRequirement};

use super::super::{BodyDoc, Param, ParamIn, ResponseDoc};
use crate::auth::SchemeDoc;
use crate::build_error::{BuildError, Diagnostic};
use crate::router::ResolvedRoute;

pub(super) fn build_param(param: Param) -> utoipa::openapi::path::Parameter {
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

pub(super) fn build_body(body: BodyDoc) -> RequestBody {
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

pub(super) fn build_response(response: ResponseDoc) -> Response {
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

pub(super) fn merge_response(
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
                    ResponseDoc::problem(
                        response.status,
                        super::super::canonical_reason(response.status),
                    ),
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

pub(super) fn requirement(auth: &[crate::auth::AuthDoc]) -> SecurityRequirement {
    let mut requirement = SecurityRequirement::default();
    for doc in auth {
        requirement = requirement.add(doc.name, doc.scopes.iter().copied());
    }
    requirement
}

/// Store the first declaration of a scheme; a second, different kind is a build error that
/// points at both declarations.
pub(super) fn merge_scheme(
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
