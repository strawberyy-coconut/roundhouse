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

/// The responses a success type can produce.
///
/// This is the content contract for a success response. Implement it for a custom response type
/// to document it exactly as the built-ins do: the status, content type, body schema and headers
/// all come from this impl. [`DocumentedError`] is the failure counterpart; `Result<T, E>`
/// combines both.
///
/// Built-ins include `Json<T>`, [`Status<CODE, T>`](crate::status::Status) for an exact code with
/// a JSON body, [`Empty<CODE>`](crate::status::Empty) for an exact code with no body, and the
/// generated named wrappers in [`crate::status`] (`Created`, `NotFound`, …).
///
/// A declaration such as `#[handler(response(201, Json<Note>))]` overrides only the status; the
/// body, content type and headers still come from the type.
///
/// A custom impl in an application crate should be marked `#[roundhouse::build_only]` so a
/// production build that compiles only `contract` drops it.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not document a response",
    label = "this return type has no `DocumentedResponse` implementation",
    note = "implement `roundhouse::doc::DocumentedResponse` for it, name a documented type such as `Json<T>`, or declare the response with `#[handler(response(status, Type))]`"
)]
pub trait DocumentedResponse {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}

/// The responses an error type can produce.
///
/// This is the failure counterpart of [`DocumentedResponse`]. A handler that returns
/// `Result<T, E>` documents `T`'s success responses and `E`'s failures; `E` may be
/// [`Problem`](crate::Problem), a custom error enum, or any other type that implements
/// `IntoResponse` and this trait. [`Status<CODE, T>`](crate::status::Status),
/// [`Empty<CODE>`](crate::status::Empty) and the generated wrappers in [`crate::status`] implement
/// both traits, so they can be either side of `Result`.
///
/// The document describes the type that leaves the handler. If `?` converts a specific error into
/// a broader one, the broader type's impl is what gets documented; return the specific error type
/// when its statuses matter.
///
/// ```ignore
/// async fn show(...) -> Result<impl IntoResponse, NoteMissing> {
///     let note = load(id)?;   // ? propagates NoteMissing; its impl supplies the failures
///     Ok(Json(note))
/// }
/// ```
///
/// A custom impl in an application crate should be marked `#[roundhouse::build_only]` so a
/// production build that compiles only `contract` drops it.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not document an error response",
    label = "this error type has no `DocumentedError` implementation",
    note = "implement `roundhouse::doc::DocumentedError` for it, return `Problem`, or declare the responses with `#[handler(response(status, Type))]`"
)]
pub trait DocumentedError {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}

impl<T: ApiSchema> DocumentedResponse for axum::Json<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<T>(components, 200)
    }
}

impl DocumentedResponse for String {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![plain_text_response(200)]
    }
}

impl DocumentedResponse for &'static str {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![plain_text_response(200)]
    }
}

impl DocumentedResponse for bytes::Bytes {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![bytes_response(200)]
    }
}

impl DocumentedResponse for axum::body::Body {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![bytes_response(200)]
    }
}

/// A fully erased response has no recoverable shape. It documents nothing, so a handler that
/// returns `Response` must declare its statuses.
impl DocumentedResponse for axum::response::Response {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

/// A redirect chooses its status at runtime (`303`, `307` or `308`), so the document says
/// `default`.
impl DocumentedResponse for axum::response::Redirect {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::default_response()]
    }
}

/// A raw `Vec<u8>` is an octet-stream body, not a JSON array. JSON arrays use `Json<Vec<T>>`.
impl DocumentedResponse for Vec<u8> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![bytes_response(200)]
    }
}

impl DocumentedResponse for axum::http::StatusCode {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::default_response()]
    }
}

impl DocumentedResponse for () {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::new(200, "OK")]
    }
}

/// An `Option<T>` does not itself promise a response shape, so the macro cannot claim a success
/// status. Return a concrete response type or declare the statuses with `response(..)`.
impl<T: DocumentedResponse> DocumentedResponse for Option<T> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

impl DocumentedResponse for std::convert::Infallible {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

impl DocumentedError for std::convert::Infallible {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

impl<T: DocumentedResponse, E: DocumentedError> DocumentedResponse for Result<T, E> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        let mut responses = T::responses(components);
        responses.extend(E::responses(components));
        responses
    }
}

/// A runtime `StatusCode` is not part of the type, so the document can only describe OpenAPI's
/// `default`. Use [`Status<CODE, T>`](crate::status::Status) or
/// [`Empty<CODE>`](crate::status::Empty) when the status is known at compile time.
impl<T: DocumentedResponse> DocumentedResponse for (axum::http::StatusCode, T) {
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

impl<T: ApiSchema> DocumentedResponse for axum::response::Html<T> {
    fn responses(components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![text_response::<T>(components, 200, "text/html")]
    }
}

impl<T: crate::attachment::AttachmentType> DocumentedResponse for crate::attachment::Attachment<T> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::attachment_response::<T>(200)]
    }
}

impl DocumentedResponse for crate::attachment::File {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::file_response(200)]
    }
}

impl<E> DocumentedResponse for axum::response::sse::Sse<E> {
    fn responses(_components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![crate::attachment::sse_response(200)]
    }
}
