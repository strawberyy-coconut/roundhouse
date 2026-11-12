use super::{ApiSchema, ResponseDoc, SchemaContext, canonical_reason, register_to_schema, schema};

pub fn json_response<T: ApiSchema>(
    components: &mut dyn SchemaContext,
    status: u16,
) -> Vec<ResponseDoc> {
    T::register(components);
    let mut response = ResponseDoc::json(status, T::schema());
    if let Some(description) = T::description() {
        response.description = description.to_string();
    }
    vec![response]
}

/// A JSON response documented from `ToSchema` alone, without roundhouse hooks.
pub fn json_response_to_schema<T: utoipa::ToSchema>(
    components: &mut dyn SchemaContext,
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

pub fn json_response_for<T>(components: &mut dyn SchemaContext, status: u16) -> Vec<ResponseDoc>
where
    T: ApiSchema,
{
    json_response::<T>(components, status)
}

/// One JSON response, for an `@responses: 200: Post` declaration.
pub fn json_response_doc<T: ApiSchema>(
    components: &mut dyn SchemaContext,
    status: u16,
) -> ResponseDoc {
    T::register(components);
    let mut response = ResponseDoc::json(status, T::schema());
    if let Some(description) = T::description() {
        response.description = description.to_string();
    }
    response
}

/// An `@responses: status: Type` declaration for a plain `ToSchema` type.
pub fn json_response_doc_to_schema<T: utoipa::ToSchema>(
    components: &mut dyn SchemaContext,
    status: u16,
) -> ResponseDoc {
    register_to_schema::<T>(components);
    ResponseDoc::json(status, <T as utoipa::PartialSchema>::schema())
}

/// Look up responses declared by `#[derive(ApiResponse)]` / `#[derive(ApiError)]`.
///
/// A hand-written `IntoResponse` type that does not implement [`ApiResponse`] contributes no
/// responses; the build then reports it as undocumented instead of failing to compile.
pub fn stored_responses_for<T: ApiResponse>(
    components: &mut dyn SchemaContext,
) -> Vec<ResponseDoc> {
    <T as ApiResponse>::responses(components)
}

pub fn text_response<T: ApiSchema>(
    components: &mut dyn SchemaContext,
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
    components: &mut dyn SchemaContext,
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
    /// The success statuses this response guarantees. An empty slice means the type is known but
    /// produces no success response; `&[0]` means the status is chosen at runtime and documented
    /// as OpenAPI's `default` response.
    const SUCCESS: &'static [u16] = &[200];

    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}

/// Error-shaped response enum. Automatic once the type is an error, `IntoResponse` and `ApiResponse`.
pub trait ApiError: std::error::Error + axum::response::IntoResponse + ApiResponse {}

impl<T> ApiError for T where T: std::error::Error + axum::response::IntoResponse + ApiResponse {}

impl<T: ApiSchema> ApiResponse for axum::Json<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<T>(components, 200)
    }
}

impl ApiResponse for String {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![plain_text_response(200)]
    }
}

impl ApiResponse for &'static str {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![plain_text_response(200)]
    }
}

impl ApiResponse for bytes::Bytes {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![bytes_response(200)]
    }
}

impl ApiResponse for axum::body::Body {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![bytes_response(200)]
    }
}

impl<T: ApiSchema> ApiResponse for Vec<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<Vec<T>>(components, 200)
    }
}

impl ApiResponse for axum::http::StatusCode {
    const SUCCESS: &'static [u16] = &[0];

    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::default_response()]
    }
}

impl ApiResponse for () {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::new(200, "OK")]
    }
}

impl<T: ApiResponse> ApiResponse for Option<T> {
    const SUCCESS: &'static [u16] = &[];

    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

impl ApiResponse for std::convert::Infallible {
    const SUCCESS: &'static [u16] = &[];

    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

impl<T: ApiResponse, E: ApiResponse> ApiResponse for Result<T, E> {
    const SUCCESS: &'static [u16] = T::SUCCESS;

    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        let mut responses = T::responses(components);
        responses.extend(E::responses(components));
        responses
    }
}

impl<T: ApiResponse> ApiResponse for (axum::http::StatusCode, T) {
    const SUCCESS: &'static [u16] = &[0];

    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        let mut responses = T::responses(components);
        if responses.is_empty() {
            return vec![ResponseDoc::default_response()];
        }
        for response in &mut responses {
            response.status = 0;
        }
        responses
    }
}

impl<T: ApiSchema> ApiResponse for axum::response::Html<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![text_response::<T>(components, 200, "text/html")]
    }
}

impl<T: crate::attachment::AttachmentType> ApiResponse for crate::attachment::Attachment<T> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::attachment_response::<T>(200)]
    }
}

impl ApiResponse for crate::attachment::File {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::file_response(200)]
    }
}

impl<E> ApiResponse for axum::response::sse::Sse<E> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::sse_response(200)]
    }
}
