use utoipa::openapi::schema::Components;

use super::{ApiSchema, ResponseDoc, canonical_reason, register_to_schema, schema};

pub fn json_response<T: ApiSchema>(components: &mut Components, status: u16) -> Vec<ResponseDoc> {
    T::register(components);
    vec![ResponseDoc::json(status, T::schema())]
}

/// A JSON response documented from `ToSchema` alone, without roundhouse hooks.
pub fn json_response_to_schema<T: utoipa::ToSchema>(
    components: &mut Components,
    status: u16,
) -> Vec<ResponseDoc> {
    register_to_schema::<T>(components);
    vec![ResponseDoc::json(
        status,
        <T as utoipa::PartialSchema>::schema(),
    )]
}

pub fn json_failures() -> Vec<ResponseDoc> {
    vec![
        ResponseDoc::problem(400, "Bad Request"),
        ResponseDoc::problem(415, "Unsupported Media Type"),
    ]
}

pub fn form_failures() -> Vec<ResponseDoc> {
    vec![
        ResponseDoc::problem(400, "Bad Request"),
        ResponseDoc::problem(415, "Unsupported Media Type"),
    ]
}

pub fn path_failures() -> Vec<ResponseDoc> {
    vec![ResponseDoc::problem(400, "Bad Request")]
}

pub fn query_failures() -> Vec<ResponseDoc> {
    vec![ResponseDoc::problem(400, "Bad Request")]
}

pub fn auth_failures<A: crate::auth::AuthScheme>() -> Vec<ResponseDoc> {
    let mut failures = vec![ResponseDoc::problem(
        A::UNAUTHENTICATED.as_u16(),
        "Unauthorized",
    )];
    if let Some(status) = A::FORBIDDEN {
        failures.push(ResponseDoc::problem(status.as_u16(), "Forbidden"));
    }
    failures
}

pub fn extension_failures() -> Vec<ResponseDoc> {
    vec![ResponseDoc::problem(500, "Internal Server Error")]
}

pub fn json_response_for<T>(components: &mut Components, status: u16) -> Vec<ResponseDoc>
where
    T: ApiSchema,
{
    json_response::<T>(components, status)
}

/// One JSON response, for a `responses(200 = Post)` declaration.
pub fn json_response_doc<T: ApiSchema>(components: &mut Components, status: u16) -> ResponseDoc {
    T::register(components);
    ResponseDoc::json(status, T::schema())
}

/// A `responses(status = Type)` declaration for a plain `ToSchema` type.
pub fn json_response_doc_to_schema<T: utoipa::ToSchema>(
    components: &mut Components,
    status: u16,
) -> ResponseDoc {
    register_to_schema::<T>(components);
    ResponseDoc::json(status, <T as utoipa::PartialSchema>::schema())
}

/// Look up responses declared by `#[derive(ApiResponse)]` / `#[derive(ApiError)]`.
///
/// A hand-written `IntoResponse` type that does not implement [`ApiResponse`] contributes no
/// responses; the build then reports it as undocumented instead of failing to compile.
pub fn stored_responses_for<T: ApiResponse>(components: &mut Components) -> Vec<ResponseDoc> {
    <T as ApiResponse>::responses(components)
}

/// Type-driven probe for a response type's documented responses.
///
/// The handler macro knows `E` in `Result<T, E>` (or the concrete return type), but it cannot
/// require `ApiResponse`: a hand-written `IntoResponse` type is legal and simply has no registered
/// documentation. The autoref fallback keeps that path compiling.
pub struct ResponseProbe<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> ResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for ResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeResponses {
    fn probe_responses(self, components: &mut Components) -> Vec<ResponseDoc>;
}

impl<T: ApiResponse + ?Sized> ProbeResponses for &ResponseProbe<T> {
    fn probe_responses(self, components: &mut Components) -> Vec<ResponseDoc> {
        T::responses(components)
    }
}

impl<T: ?Sized> ProbeResponses for &&ResponseProbe<T> {
    fn probe_responses(self, _components: &mut Components) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

pub fn text_response<T: ApiSchema>(
    components: &mut Components,
    status: u16,
    content_type: &str,
) -> ResponseDoc {
    T::register(components);
    ResponseDoc {
        status,
        description: canonical_reason(status).to_string(),
        content_type: Some(content_type.to_string()),
        schema: Some(T::schema()),
        headers: Vec::new(),
    }
}

/// A non-JSON response body documented from `ToSchema` alone.
pub fn text_response_to_schema<T: utoipa::ToSchema>(
    components: &mut Components,
    status: u16,
    content_type: &str,
) -> ResponseDoc {
    register_to_schema::<T>(components);
    ResponseDoc {
        status,
        description: canonical_reason(status).to_string(),
        content_type: Some(content_type.to_string()),
        schema: Some(<T as utoipa::PartialSchema>::schema()),
        headers: Vec::new(),
    }
}

pub fn plain_text_response(status: u16) -> ResponseDoc {
    ResponseDoc {
        status,
        description: canonical_reason(status).to_string(),
        content_type: Some("text/plain; charset=utf-8".to_string()),
        schema: Some(schema::string()),
        headers: Vec::new(),
    }
}

pub fn bytes_response(status: u16) -> ResponseDoc {
    ResponseDoc {
        status,
        description: canonical_reason(status).to_string(),
        content_type: Some("application/octet-stream".to_string()),
        schema: Some(schema::string_with_format(
            utoipa::openapi::schema::SchemaFormat::KnownFormat(
                utoipa::openapi::schema::KnownFormat::Binary,
            ),
        )),
        headers: Vec::new(),
    }
}

/// Implemented by `#[derive(ApiResponse)]` and `#[derive(ApiError)]`, and by [`Problem`].
///
/// [`Problem`]: crate::Problem
pub trait ApiResponse {
    fn responses(components: &mut Components) -> Vec<ResponseDoc>;
}

/// Error-shaped response enum. Automatic once the type is an error, `IntoResponse` and `ApiResponse`.
pub trait ApiError: std::error::Error + axum::response::IntoResponse + ApiResponse {}

impl<T> ApiError for T where T: std::error::Error + axum::response::IntoResponse + ApiResponse {}
