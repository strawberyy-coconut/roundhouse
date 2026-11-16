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

/// One JSON response, for a `response(200, Post)` declaration.
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

/// A `response(status, Type)` declaration for a plain `ToSchema` type.
pub fn json_response_doc_to_schema<T: utoipa::ToSchema>(
    components: &mut dyn SchemaContext,
    status: u16,
) -> ResponseDoc {
    register_to_schema::<T>(components);
    ResponseDoc::json(status, <T as utoipa::PartialSchema>::schema())
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

/// The internal response-documentation contract for built-in response types.
///
/// This is macro plumbing, not an extension point. It is hidden from docs and removed from the
/// public prelude; application code returns plain `impl IntoResponse` or declares a typed
/// response with `#[handler(response(..))]`.
#[doc(hidden)]
pub trait ResponseDocSource {
    /// The success statuses this response guarantees. An empty slice means the type is known but
    /// produces no success response; `&[0]` means the status is chosen at runtime and documented
    /// as OpenAPI's `default` response.
    const SUCCESS: &'static [u16] = &[200];

    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}

impl<T: ApiSchema> ResponseDocSource for axum::Json<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<T>(components, 200)
    }
}

impl ResponseDocSource for String {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![plain_text_response(200)]
    }
}

impl ResponseDocSource for &'static str {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![plain_text_response(200)]
    }
}

impl ResponseDocSource for bytes::Bytes {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![bytes_response(200)]
    }
}

impl ResponseDocSource for axum::body::Body {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![bytes_response(200)]
    }
}

impl<T: ApiSchema> ResponseDocSource for Vec<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<Vec<T>>(components, 200)
    }
}

impl ResponseDocSource for axum::http::StatusCode {
    const SUCCESS: &'static [u16] = &[0];

    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::default_response()]
    }
}

impl ResponseDocSource for () {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::new(200, "OK")]
    }
}

/// An `Option<T>` does not itself promise a response shape, so the macro cannot claim a success
/// status. Return a concrete response type or declare the statuses with `response(..)`.
impl<T: ResponseDocSource> ResponseDocSource for Option<T> {
    const SUCCESS: &'static [u16] = &[];

    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

impl ResponseDocSource for std::convert::Infallible {
    const SUCCESS: &'static [u16] = &[];

    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

impl<T: ResponseDocSource, E: ResponseDocSource> ResponseDocSource for Result<T, E> {
    const SUCCESS: &'static [u16] = T::SUCCESS;

    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        let mut responses = T::responses(components);
        responses.extend(E::responses(components));
        responses
    }
}

impl<T: ResponseDocSource> ResponseDocSource for (axum::http::StatusCode, T) {
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

impl<T: ApiSchema> ResponseDocSource for axum::response::Html<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![text_response::<T>(components, 200, "text/html")]
    }
}

impl<T: crate::attachment::AttachmentType> ResponseDocSource for crate::attachment::Attachment<T> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::attachment_response::<T>(200)]
    }
}

impl ResponseDocSource for crate::attachment::File {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::file_response(200)]
    }
}

impl<E> ResponseDocSource for axum::response::sse::Sse<E> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::sse_response(200)]
    }
}
