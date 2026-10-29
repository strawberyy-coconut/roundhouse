//! OpenAPI: utoipa's document model, roundhouse's derivation.

pub mod build;
pub mod downgrade;
pub mod response;
pub use build::{build, build_with_options};
pub mod layer;

/// Schema derivation and constraint helpers, re-exported from `roundhouse-schema`.
pub use roundhouse_schema::schema;

use std::any::TypeId;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, Schema};

pub use roundhouse_schema::{ApiSchema, Param, ParamIn, PathNames, PathValidate};
pub use utoipa;

/// A documented request body.
#[derive(Clone)]
pub struct BodyDoc {
    pub content_type: String,
    pub schema: RefOr<Schema>,
    pub required: bool,
    pub description: Option<String>,
}

/// A documented response header.
#[derive(Clone)]
pub struct HeaderDoc {
    pub name: String,
    pub schema: RefOr<Schema>,
    pub required: bool,
    pub append: bool,
}

/// One documented response.
#[derive(Clone)]
pub struct ResponseDoc {
    pub status: u16,
    pub description: String,
    pub content_type: Option<String>,
    pub schema: Option<RefOr<Schema>>,
    pub headers: Vec<HeaderDoc>,
}

impl ResponseDoc {
    pub fn new(status: u16, description: impl Into<String>) -> Self {
        Self {
            status,
            description: description.into(),
            content_type: None,
            schema: None,
            headers: Vec::new(),
        }
    }

    pub fn json(status: u16, schema: RefOr<Schema>) -> Self {
        Self {
            status,
            description: canonical_reason(status).to_string(),
            content_type: Some("application/json".to_string()),
            schema: Some(schema),
            headers: Vec::new(),
        }
    }

    pub fn problem(status: u16, title: &str) -> Self {
        Self {
            status,
            description: title.to_string(),
            content_type: Some("application/problem+json".to_string()),
            schema: Some(problem_schema()),
            headers: Vec::new(),
        }
    }
}

/// The documentation half a `#[handler]` attribute derives from the signature.
#[derive(Clone, Default)]
pub struct HandlerDocs {
    pub params: Vec<Param>,
    pub body: Option<BodyDoc>,
    pub failures: Vec<ResponseDoc>,
    pub responses: Vec<ResponseDoc>,
    pub required_auth: Vec<crate::auth::AuthDoc>,
    pub optional_auth: Vec<crate::auth::AuthDoc>,
    pub schemes: Vec<crate::auth::SchemeDoc>,
}

/// Implemented by the marker type `#[handler]` emits beside each handler.
pub trait HandlerMeta: Send + Sync + 'static {
    /// The state type read from `State<T>`, or `()` for a stateless handler.
    type State: 'static;

    /// Whether a `State<T>` extractor appeared in the signature.
    const HAS_STATE: bool;

    /// Capture names declared by the handler's `Path<T>` extractor, in template order.
    const PATH_NAMES: &'static [&'static str] = &[];

    fn docs(&self, components: &mut Components) -> HandlerDocs;
}

pub fn handler_state<M: HandlerMeta>() -> Option<TypeId> {
    if M::HAS_STATE {
        Some(TypeId::of::<M::State>())
    } else {
        None
    }
}

/// The Problem body as a schema, for documented rejection responses.
pub fn problem_schema() -> RefOr<Schema> {
    use utoipa::openapi::schema::{ArrayBuilder, ObjectBuilder, SchemaFormat, Type};

    let error = ObjectBuilder::new()
        .schema_type(Type::Object)
        .property("pointer", schema::string())
        .property("code", schema::string())
        .property("message", schema::string())
        .property("limit", schema::any_value())
        .required("pointer")
        .required("code")
        .required("message")
        .build();

    ObjectBuilder::new()
        .schema_type(Type::Object)
        .property(
            "type",
            schema::string_with_format(SchemaFormat::Custom("uri".to_string())),
        )
        .property("title", schema::string())
        .property(
            "status",
            ObjectBuilder::new()
                .schema_type(Type::Integer)
                .minimum(Some(100u16))
                .maximum(Some(599u16)),
        )
        .property("detail", schema::nullable(schema::string()))
        .property("instance", schema::nullable(schema::string()))
        .property("errors", ArrayBuilder::new().items(error))
        .required("type")
        .required("title")
        .required("status")
        .into()
}

/// Resolve a component name. Unique type names stay simple; collisions get the module path
/// (`blog.Post` style) so two `Post`s from different modules cannot overwrite each other.
pub fn component_name(simple: &'static str, module: &'static str) -> String {
    use std::collections::HashMap;
    use std::sync::OnceLock;

    static COUNTS: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    let counts = COUNTS.get_or_init(|| {
        let mut counts = HashMap::new();
        for record in inventory::iter::<crate::registry::ComponentTypeRecord> {
            *counts.entry(record.simple).or_insert(0) += 1;
        }
        counts
    });

    if counts.get(simple).copied().unwrap_or(1) > 1 {
        format!("{}.{}", module.replace("::", "."), simple)
    } else {
        simple.to_string()
    }
}

pub fn canonical_reason(status: u16) -> &'static str {
    http::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("Unknown")
}

/// Register the schemas and build the response for a JSON body.
pub fn json_body<T: ApiSchema>(components: &mut Components) -> BodyDoc {
    T::register(components);
    BodyDoc {
        content_type: "application/json".to_string(),
        schema: T::schema(),
        required: true,
        description: None,
    }
}

pub fn multipart_body<F: crate::multipart::MultipartForm>(components: &mut Components) -> BodyDoc {
    BodyDoc {
        content_type: "multipart/form-data".to_string(),
        schema: F::schema(components),
        required: true,
        description: None,
    }
}

pub fn multipart_failures() -> Vec<ResponseDoc> {
    vec![
        ResponseDoc::problem(400, "Bad Request"),
        ResponseDoc::problem(413, "Payload Too Large"),
        ResponseDoc::problem(415, "Unsupported Media Type"),
    ]
}

pub fn form_body<T: ApiSchema>(components: &mut Components) -> BodyDoc {
    T::register(components);
    BodyDoc {
        content_type: "application/x-www-form-urlencoded".to_string(),
        schema: T::schema(),
        required: true,
        description: None,
    }
}

pub fn json_response<T: ApiSchema>(components: &mut Components, status: u16) -> Vec<ResponseDoc> {
    T::register(components);
    vec![ResponseDoc::json(status, T::schema())]
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

pub fn path_params<T: PathNames>() -> Vec<Param> {
    T::params()
}

/// Query parameters declared by `#[derive(QueryParams)]`.
pub trait QueryParams {
    fn params(components: &mut Components) -> Vec<Param>;

    fn validate_at(&self, _pointer: &str, _errors: &mut crate::validate::ValidationErrors) {}

    fn validate(&self) -> Result<(), crate::validate::ValidationErrors> {
        let mut errors = crate::validate::ValidationErrors::new();
        self.validate_at("", &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

pub fn stored_query_params_for<T: ?Sized>(components: &mut Components) -> Vec<Param> {
    let key = std::any::type_name::<T>();
    inventory::iter::<crate::registry::QueryDocRecord>
        .into_iter()
        .find(|record| (record.type_key)() == key)
        .map(|record| (record.params)(components))
        .unwrap_or_default()
}

pub fn json_response_for<T>(components: &mut Components, status: u16) -> Vec<ResponseDoc>
where
    T: ApiSchema,
{
    json_response::<T>(components, status)
}

/// Look up responses registered by `#[derive(ApiResponse)]` / `#[derive(ApiError)]`.
pub fn stored_responses_for<T: ?Sized>(components: &mut Components) -> Vec<ResponseDoc> {
    let key = std::any::type_name::<T>();
    inventory::iter::<crate::registry::ResponseRecord>
        .into_iter()
        .find(|record| (record.type_key)() == key)
        .map(|record| (record.docs)(components))
        .unwrap_or_default()
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

/// Implemented by `#[derive(ApiResponse)]` and `#[derive(ApiError)]`.
pub trait ApiResponse {
    fn responses(components: &mut Components) -> Vec<ResponseDoc>;
}

/// Error-shaped response enum. Automatic once the type is an error, `IntoResponse` and `ApiResponse`.
pub trait ApiError: std::error::Error + axum::response::IntoResponse + ApiResponse {}

impl<T> ApiError for T where T: std::error::Error + axum::response::IntoResponse + ApiResponse {}
