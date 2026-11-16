//! Response verification from the build-time contract.
//!
//! Requests are validated in production from `contract.json`. Tests use the same response table
//! to check that a handler's actual status, content type and JSON body match what was documented.
//! This module has no `utoipa` dependency, so `cargo roundhouse verify` works in a
//! `contract-runtime`-only build.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{MatchedPath, Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use roundhouse_schema::engine::{SchemaError, compile_value, rewrite_refs, validate_with};
use roundhouse_schema::jsonschema::Validator;
use serde_json::Value;

use crate::problem::Problem;

const MAX_RESPONSE_BODY: usize = 32 * 1024 * 1024;

struct ResponsePlan {
    content_type: Option<String>,
    validator: Option<Validator>,
}

/// Precompiled response plans keyed by operation id and status.
pub struct ContractResponseValidator {
    by_status: HashMap<(String, u16), ResponsePlan>,
    default: HashMap<String, ResponsePlan>,
    /// `(METHOD, path)` to operation id, for middleware that only has the matched route.
    operations: HashMap<(String, String), String>,
}

impl ContractResponseValidator {
    /// Compile the response plans from a `contract.json`.
    pub fn from_contract_json(contract: &str) -> Result<Self, SchemaError> {
        let contract: Value = serde_json::from_str(contract)?;
        let mut defs = contract
            .pointer("/validation/defs")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        rewrite_refs(&mut defs);

        let mut validator = Self {
            by_status: HashMap::new(),
            default: HashMap::new(),
            operations: HashMap::new(),
        };
        let Some(operations) = contract["operations"].as_array() else {
            return Ok(validator);
        };
        for operation in operations {
            let Some(operation_id) = operation["operation_id"].as_str() else {
                continue;
            };
            if let (Some(method), Some(path)) =
                (operation["method"].as_str(), operation["path"].as_str())
            {
                validator.operations.insert(
                    (
                        method.to_ascii_uppercase(),
                        normalize_path(path).to_string(),
                    ),
                    operation_id.to_string(),
                );
            }
            let Some(responses) = operation["responses"].as_array() else {
                continue;
            };
            for response in responses {
                let status = response["status"].as_u64().unwrap_or(0) as u16;
                let (content_type, schema) = first_content(response);
                let compiled = schema
                    .as_ref()
                    .map(|schema| compile_value(schema, &defs))
                    .transpose()?;
                let plan = ResponsePlan {
                    content_type,
                    validator: compiled,
                };
                if status == 0 {
                    validator.default.insert(operation_id.to_string(), plan);
                } else {
                    validator
                        .by_status
                        .insert((operation_id.to_string(), status), plan);
                }
            }
        }
        Ok(validator)
    }

    /// The operation id for a matched method and path.
    pub fn operation_id(&self, method: &http::Method, path: &str) -> Option<&str> {
        self.operations
            .get(&(
                method.as_str().to_ascii_uppercase(),
                normalize_path(path).to_string(),
            ))
            .map(String::as_str)
    }

    /// Validate one actual response for `operation_id`, returning it with its body intact.
    pub async fn validate_response(
        &self,
        operation_id: &str,
        response: Response,
    ) -> Result<Response, String> {
        let (parts, bytes) = self.check(operation_id, response).await?;
        Ok(Response::from_parts(parts, Body::from(bytes)))
    }

    /// Validate one actual response for `operation_id`.
    pub async fn validate(&self, operation_id: &str, response: Response) -> Result<(), String> {
        self.check(operation_id, response).await.map(|_| ())
    }

    async fn check(
        &self,
        operation_id: &str,
        response: Response,
    ) -> Result<(http::response::Parts, Bytes), String> {
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let plan = self
            .by_status
            .get(&(operation_id.to_string(), status))
            .or_else(|| self.default.get(operation_id))
            .ok_or_else(|| {
                format!("operation `{operation_id}` does not document status {status}")
            })?;

        if let Some(expected) = &plan.content_type {
            let expected = media_type(expected);
            let actual = content_type.as_deref().map(media_type).unwrap_or("<none>");
            if expected != actual {
                return Err(format!(
                    "operation `{operation_id}` status {status} content type is `{actual}`, \
                     documented as `{expected}`"
                ));
            }
        }

        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, MAX_RESPONSE_BODY)
            .await
            .map_err(|error| format!("could not read response body: {error}"))?;
        if let Some(validator) = &plan.validator {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|error| format!("response body is not JSON: {error}"))?;
            validate_with(validator, &value)
                .map_err(|errors| format!("response body violates the contract: {errors}"))?;
        }
        Ok((parts, bytes))
    }

    /// Validate a response by reading a body that is already available.
    pub fn validate_value(
        &self,
        operation_id: &str,
        status: u16,
        content_type: Option<&str>,
        value: &Value,
    ) -> Result<(), String> {
        let plan = self
            .by_status
            .get(&(operation_id.to_string(), status))
            .or_else(|| self.default.get(operation_id))
            .ok_or_else(|| {
                format!("operation `{operation_id}` does not document status {status}")
            })?;
        if let Some(expected) = &plan.content_type {
            let expected = media_type(expected);
            let actual = content_type.map(media_type).unwrap_or("<none>");
            if expected != actual {
                return Err(format!(
                    "operation `{operation_id}` status {status} content type is `{actual}`, \
                     documented as `{expected}`"
                ));
            }
        }
        let Some(validator) = &plan.validator else {
            return Ok(());
        };
        validate_with(validator, value)
            .map_err(|errors| format!("response body violates the contract: {errors}"))
    }
}

fn media_type(value: &str) -> &str {
    value.split(';').next().unwrap_or(value).trim()
}

fn normalize_path(path: &str) -> &str {
    if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    }
}

fn first_content(response: &Value) -> (Option<String>, Option<Value>) {
    let Some(content) = response["content"].as_object() else {
        return (None, None);
    };
    let Some((content_type, media)) = content.iter().next() else {
        return (None, None);
    };
    (Some(content_type.clone()), media.get("schema").cloned())
}

/// Middleware that verifies actual responses against the contract.
pub async fn validate_response(
    State(validator): State<Arc<ContractResponseValidator>>,
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
    let Some(operation_id) = validator.operation_id(&method, &path) else {
        return response;
    };
    match validator.validate_response(operation_id, response).await {
        Ok(response) => response,
        Err(message) => {
            Problem::internal(format!("response verification failed: {message}")).into_response()
        }
    }
}
