//! Opt-in response validation, using the same schema engine as request validation.
//!
//! The document is already a JSON Schema document. A response entry compiles its schema once at
//! build time and the middleware validates the JSON body against it, so the checks cannot drift
//! from the keywords the emitter produces.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use roundhouse_schema::engine::{SchemaError, compile_value, defs_from_components, validate_with};
use roundhouse_schema::jsonschema::Validator;
use serde_json::Value;
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::openapi::schema::Schema;
use utoipa::openapi::{OpenApi, RefOr, Response as DocResponse};

use crate::problem::Problem;

/// Bound on the response body buffered for validation in tests and debug builds.
const MAX_RESPONSE_BODY: usize = 32 * 1024 * 1024;

#[derive(Clone)]
struct ResponseEntry {
    content_type: Option<String>,
    validator: Option<Validator>,
}

/// The documented responses for one operation, with OpenAPI's `default` kept separate.
#[derive(Default)]
struct OperationResponses {
    by_status: HashMap<u16, ResponseEntry>,
    default: Option<ResponseEntry>,
}

/// Precomputed response validators keyed by method and matched path.
pub struct ResponseValidator {
    entries: HashMap<(http::Method, String), OperationResponses>,
}

impl ResponseValidator {
    pub fn new(document: &OpenApi) -> Result<Self, SchemaError> {
        let components = document.components.clone().unwrap_or_default();
        let defs = Value::Object(defs_from_components(&components)?);
        let mut entries = HashMap::new();
        for (path, item) in &document.paths.paths {
            add_path(&mut entries, path, item, &defs, &components.responses)?;
        }
        Ok(Self { entries })
    }

    fn entry(&self, method: &http::Method, path: &str, status: u16) -> Option<&ResponseEntry> {
        let responses = self.entries.get(&(method.clone(), path.to_string()))?;
        responses
            .by_status
            .get(&status)
            .or(responses.default.as_ref())
    }
}

fn add_path(
    entries: &mut HashMap<(http::Method, String), OperationResponses>,
    path: &str,
    item: &PathItem,
    defs: &Value,
    components: &std::collections::BTreeMap<String, RefOr<DocResponse>>,
) -> Result<(), SchemaError> {
    for (method, operation) in [
        (http::Method::GET, &item.get),
        (http::Method::POST, &item.post),
        (http::Method::PUT, &item.put),
        (http::Method::PATCH, &item.patch),
        (http::Method::DELETE, &item.delete),
        (http::Method::HEAD, &item.head),
        (http::Method::OPTIONS, &item.options),
        (http::Method::TRACE, &item.trace),
    ] {
        if let Some(operation) = operation {
            entries.insert(
                (method, path.to_string()),
                response_entries(operation, defs, components)?,
            );
        }
    }
    Ok(())
}

fn response_entries(
    operation: &Operation,
    defs: &Value,
    components: &std::collections::BTreeMap<String, RefOr<DocResponse>>,
) -> Result<OperationResponses, SchemaError> {
    let mut entries = OperationResponses::default();
    for (status, response) in &operation.responses.responses {
        let Some(response) = resolve_response(response, components) else {
            continue;
        };
        let (content_type, schema) = first_content(response);
        let validator = match schema {
            Some(schema) => {
                let value = serde_json::to_value(&schema)?;
                Some(compile_value(&value, defs)?)
            }
            None => None,
        };
        let entry = ResponseEntry {
            content_type,
            validator,
        };
        if status == "default" {
            entries.default = Some(entry);
        } else if let Ok(status) = status.parse::<u16>() {
            entries.by_status.insert(status, entry);
        }
    }
    Ok(entries)
}

fn resolve_response<'a>(
    response: &'a RefOr<DocResponse>,
    components: &'a std::collections::BTreeMap<String, RefOr<DocResponse>>,
) -> Option<&'a DocResponse> {
    match response {
        RefOr::T(response) => Some(response),
        RefOr::Ref(reference) => {
            let name = reference
                .ref_location
                .strip_prefix("#/components/responses/")?;
            match components.get(name)? {
                RefOr::T(response) => Some(response),
                RefOr::Ref(_) => None,
            }
        }
    }
}

fn first_content(response: &DocResponse) -> (Option<String>, Option<RefOr<Schema>>) {
    response
        .content
        .iter()
        .next()
        .and_then(|(content_type, content)| match content {
            RefOr::T(content) => Some((Some(content_type.clone()), content.schema.clone())),
            RefOr::Ref(_) => None,
        })
        .unwrap_or((None, None))
}

/// Middleware installed by `App::validate_responses()`.
pub async fn validate_response(
    State(validator): State<Arc<ResponseValidator>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let matched = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_string());
    let response = next.run(request).await;

    let Some(path) = matched else {
        return response;
    };
    let Some(entry) = validator.entry(&method, &path, response.status().as_u16()) else {
        return response;
    };
    let Some(schema_validator) = &entry.validator else {
        return response;
    };

    let is_json = entry
        .content_type
        .as_deref()
        .is_some_and(|content_type| content_type.contains("json"));
    if !is_json {
        return response;
    }

    let actual_content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    if let Some(expected) = &entry.content_type
        && !actual_content_type.starts_with(expected)
    {
        return Problem::internal(format!(
            "response validation failed: expected `{expected}`, got `{actual_content_type}`"
        ))
        .into_response();
    }

    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_RESPONSE_BODY).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return Problem::internal(format!(
                "response validation failed: could not read body: {error}"
            ))
            .into_response();
        }
    };

    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            return Problem::internal(format!(
                "response validation failed: body is not JSON: {error}"
            ))
            .into_response();
        }
    };

    if let Err(errors) = validate_with(schema_validator, &value) {
        return Problem::internal(format!("response validation failed: {errors}")).into_response();
    }

    Response::from_parts(parts, Body::from(bytes))
}
