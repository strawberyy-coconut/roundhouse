//! Opt-in response validation against the generated document.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use utoipa::openapi::path::{Operation, PathItem};
use utoipa::openapi::schema::{
    AdditionalProperties, ArrayItems, Components, Object, Schema, SchemaType, Type,
};
use utoipa::openapi::{OpenApi, RefOr, Response as DocResponse};

use crate::problem::Problem;

#[derive(Clone)]
struct ResponseEntry {
    content_type: Option<String>,
    schema: Option<RefOr<Schema>>,
}

/// Precomputed response schemas keyed by method and matched path.
pub struct ResponseValidator {
    entries: HashMap<(http::Method, String), HashMap<u16, ResponseEntry>>,
    components: Components,
}

impl ResponseValidator {
    pub fn new(document: &OpenApi) -> Self {
        let mut entries = HashMap::new();
        for (path, item) in &document.paths.paths {
            add_path(&mut entries, path, item);
        }
        Self {
            entries,
            components: document.components.clone().unwrap_or_default(),
        }
    }

    fn entry(&self, method: &http::Method, path: &str, status: u16) -> Option<&ResponseEntry> {
        self.entries
            .get(&(method.clone(), path.to_string()))?
            .get(&status)
    }
}

fn add_path(
    entries: &mut HashMap<(http::Method, String), HashMap<u16, ResponseEntry>>,
    path: &str,
    item: &PathItem,
) {
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
            entries.insert((method, path.to_string()), response_entries(operation));
        }
    }
}

fn response_entries(operation: &Operation) -> HashMap<u16, ResponseEntry> {
    let mut entries = HashMap::new();
    for (status, response) in &operation.responses.responses {
        let Ok(status) = status.parse::<u16>() else {
            continue;
        };
        let response = match response {
            RefOr::T(response) => Some(response.clone()),
            RefOr::Ref(_) => None,
        };
        let Some(response) = response else {
            continue;
        };
        let (content_type, schema) = first_content(&response);
        entries.insert(
            status,
            ResponseEntry {
                content_type,
                schema,
            },
        );
    }
    entries
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
    let Some(schema) = &entry.schema else {
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
    let bytes = match axum::body::to_bytes(body, 32 * 1024 * 1024).await {
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

    let mut errors = Vec::new();
    check(&value, schema, &validator.components, "", 0, &mut errors);
    if !errors.is_empty() {
        return Problem::internal(format!("response validation failed: {}", errors.join("; ")))
            .into_response();
    }

    Response::from_parts(parts, Body::from(bytes))
}

/// Validate one JSON value against one schema. Exposed for tests and tooling.
pub fn validate_value(
    value: &Value,
    schema: &RefOr<Schema>,
    components: &Components,
) -> Vec<String> {
    let mut errors = Vec::new();
    check(value, schema, components, "", 0, &mut errors);
    errors
}

fn check(
    value: &Value,
    schema: &RefOr<Schema>,
    components: &Components,
    pointer: &str,
    depth: usize,
    errors: &mut Vec<String>,
) {
    if depth > 32 {
        errors.push(format!("{pointer}: schema recursion limit"));
        return;
    }
    match schema {
        RefOr::Ref(reference) => {
            let name = reference
                .ref_location
                .rsplit('/')
                .next()
                .unwrap_or_default();
            if let Some(schema) = components.schemas.get(name) {
                check(value, schema, components, pointer, depth + 1, errors);
            }
        }
        RefOr::T(schema) => match schema {
            Schema::Object(object) => {
                check_object(value, object, components, pointer, depth, errors)
            }
            Schema::Array(array) => {
                if !type_matches(&array.schema_type, value) {
                    errors.push(format!("{pointer}: expected array"));
                }
                if let Value::Array(items) = value {
                    if let ArrayItems::RefOrSchema(item_schema) = &array.items {
                        for (index, item) in items.iter().enumerate() {
                            check(
                                item,
                                item_schema,
                                components,
                                &crate::validate::pointer(pointer, &index.to_string()),
                                depth + 1,
                                errors,
                            );
                        }
                    }
                    if let Some(min) = array.min_items
                        && items.len() < min
                    {
                        errors.push(format!("{pointer}: fewer than {min} items"));
                    }
                    if let Some(max) = array.max_items
                        && items.len() > max
                    {
                        errors.push(format!("{pointer}: more than {max} items"));
                    }
                }
                let _ = value;
            }
            Schema::OneOf(one_of) => {
                let matched = one_of.items.iter().any(|item| {
                    let mut nested = Vec::new();
                    check(value, item, components, pointer, depth + 1, &mut nested);
                    nested.is_empty()
                });
                if !matched {
                    errors.push(format!("{pointer}: matches no oneOf branch"));
                }
            }
            Schema::AllOf(all_of) => {
                for item in &all_of.items {
                    check(value, item, components, pointer, depth + 1, errors);
                }
            }
            Schema::AnyOf(any_of) => {
                let matched = any_of.items.iter().any(|item| {
                    let mut nested = Vec::new();
                    check(value, item, components, pointer, depth + 1, &mut nested);
                    nested.is_empty()
                });
                if !matched {
                    errors.push(format!("{pointer}: matches no anyOf branch"));
                }
            }
            _ => {}
        },
    }
}

fn check_object(
    value: &Value,
    object: &Object,
    components: &Components,
    pointer: &str,
    depth: usize,
    errors: &mut Vec<String>,
) {
    if !type_matches(&object.schema_type, value) {
        errors.push(format!("{pointer}: expected object"));
        return;
    }
    let Value::Object(map) = value else {
        return;
    };
    for required in &object.required {
        if !map.contains_key(required) {
            errors.push(format!("{pointer}: missing required `{required}`"));
        }
    }
    for (name, schema) in &object.properties {
        if let Some(value) = map.get(name) {
            check(
                value,
                schema,
                components,
                &crate::validate::pointer(pointer, name),
                depth + 1,
                errors,
            );
        }
    }
    if matches!(
        object.additional_properties.as_deref(),
        Some(AdditionalProperties::FreeForm(false))
    ) {
        for name in map.keys() {
            if !object.properties.contains_key(name) {
                errors.push(format!("{pointer}: unexpected `{name}`"));
            }
        }
    }
}

fn type_matches(schema_type: &SchemaType, value: &Value) -> bool {
    match schema_type {
        SchemaType::AnyValue => true,
        SchemaType::Type(Type::Null) => value.is_null(),
        SchemaType::Type(Type::Object) => value.is_object(),
        SchemaType::Type(Type::Array) => value.is_array(),
        SchemaType::Type(Type::String) => value.is_string(),
        SchemaType::Type(Type::Integer) => value.is_i64() || value.is_u64(),
        SchemaType::Type(Type::Number) => value.is_number(),
        SchemaType::Type(Type::Boolean) => value.is_boolean(),
        SchemaType::Array(types) => types.iter().any(|ty| match ty {
            Type::Null => value.is_null(),
            Type::Object => value.is_object(),
            Type::Array => value.is_array(),
            Type::String => value.is_string(),
            Type::Integer => value.is_i64() || value.is_u64(),
            Type::Number => value.is_number(),
            Type::Boolean => value.is_boolean(),
        }),
    }
}
