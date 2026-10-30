//! The documentation half: schema derivation, parameter tables and response declarations.
//!
//! Most applications use this module through the derives re-exported at the crate root:
//! `#[derive(ToSchema, ApiSchema)]`, `#[derive(QueryParams)]`, `#[derive(PathParams)]`,
//! `#[derive(ApiResponse)]` and `#[derive(ApiError)]`, plus `#[derive(Multipart)]` with the
//! `multipart` feature.
//!
//! The traits here are the manual escape hatch for types the derives cannot see:
//!
//! - [`ApiSchema`] — a type's JSON Schema together with its runtime checks.
//! - [`ApiResponse`] — the documented responses a return type produces; `ApiError` is re-exported
//!   from the macros.
//! - [`ResponseDoc`] and [`layer`] — hand-written responses and documented layer guarantees.
//! - [`validate_document`] — the structural check every build runs over the finished document.
//!
//! Schemas are derived by `utoipa::ToSchema`; `#[derive(ToSchema, ApiSchema)]` adds the runtime checks.
//! Component names therefore follow utoipa: unique names stay simple, and a collision is
//! disambiguated with `#[schema(as = module::Type)]`.

pub mod build;
pub mod downgrade;
#[cfg(feature = "schema-engine")]
pub mod response;
pub mod validity;
pub use build::{build, build_with_options};
pub use validity::validate_document;
pub mod layer;

/// Schema derivation and constraint helpers, re-exported from `roundhouse-schema`.
pub use roundhouse_schema::schema;

use std::any::TypeId;
use std::collections::BTreeMap;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, Schema};

pub use roundhouse_schema::{ApiSchema, Param, ParamIn, PathNames, PathValidate};
pub use utoipa;

/// A documented request body.
#[derive(Clone)]
pub struct BodyDoc {
    pub content_type: String,
    pub schema: RefOr<Schema>,
    /// Per-property encoding, e.g. the accepted content types of multipart file parts.
    pub encoding: BTreeMap<String, utoipa::openapi::encoding::Encoding>,
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

    /// The catch-all error response: a `Problem` at a status chosen at runtime.
    pub fn problem_default() -> Self {
        Self::problem(0, "Error")
    }

    /// A response whose status is chosen at runtime (`-> StatusCode`). The document says
    /// `default` with no body, which is the honest shape.
    pub fn default_response() -> Self {
        Self {
            status: 0,
            description: "Default Response".to_string(),
            content_type: None,
            schema: None,
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
    /// A body extractor the framework can see syntactically but cannot verify at runtime
    /// (`axum::Json`, `axum::Form`, `axum::extract::Multipart`). A route that validates the body
    /// through its shim clears this before the document is checked.
    pub unverified: bool,
    /// A response type the document cannot describe (`axum::Json<T>`, `axum::Html<T>` returned
    /// directly rather than through a derived response).
    pub unverified_response: bool,
    /// An extractor whose documented contribution the macro cannot infer (a custom `Tenant`,
    /// a bare `HeaderMap`, a typed header, ...).
    pub unverified_extractor: bool,
    /// A `Result<_, E>` whose `E` registered no documented responses.
    pub undocumented_error: Option<&'static str>,
    /// The schema of a scalar `Path<T>`; the parameter's name comes from the template.
    pub path_schema:
        Option<fn() -> Option<utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>>,
    /// The Rust type of the request body, so schema mode can compile the body's own schema even
    /// when it is composite (`Vec<T>`, `Option<T>`, maps, tuples).
    pub body_type: Option<fn() -> std::any::TypeId>,
    /// File fields declared without `max_bytes`; they fall back to the app-wide body limit.
    /// The build warns (and `strict` rejects) so the most permissive declaration is visible.
    pub unbounded_file_fields: Vec<&'static str>,
}

/// Implemented by the marker type `#[handler]` emits beside each handler.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a roundhouse handler marker",
    label = "add `#[handler]` to the handler function",
    note = "`routes!` needs the marker that `#[handler]` generates beside the handler; \
            a plain `async fn` has no handler documentation to derive"
)]
pub trait HandlerMeta: Send + Sync + 'static {
    /// The state type read from `State<T>`, or `()` for a stateless handler.
    type State: 'static;

    /// Whether a `State<T>` extractor appeared in the signature.
    const HAS_STATE: bool;

    /// Capture names declared by the handler's `Path<T>` extractor, in template order.
    const PATH_NAMES: &'static [&'static str] = &[];

    /// Scheme names the handler requires with `Auth<A>` (`Option<Auth<A>>` does not count).
    ///
    /// A mount-level guard for the same scheme is skipped: the extractor already performs the
    /// check during extraction, so the request must not be authenticated twice.
    const REQUIRED_AUTH: &'static [&'static str] = &[];

    /// The success statuses the return type guarantees, or `None` when the macro cannot classify
    /// it (a custom `ApiResponse`). `Some(&[])` means the macro knows there is no success
    /// response, so a route entry must declare one.
    const INFERRED_SUCCESS: Option<&'static [u16]> = None;

    fn docs(components: &mut Components) -> HandlerDocs;

    /// Documentation with the route's validation policy.
    ///
    /// The generated implementation clears the body marker when the route's shim validated the
    /// body, so `strict()` does not reject a body the router enforces. A hand-written
    /// implementation can ignore the flag.
    fn docs_with(components: &mut Components, _validate: bool) -> HandlerDocs {
        Self::docs(components)
    }
}

pub fn handler_state<M: HandlerMeta>() -> Option<TypeId> {
    if M::HAS_STATE {
        Some(TypeId::of::<M::State>())
    } else {
        None
    }
}

/// Probe used by the handler macro to document a foreign `axum::Json<T>` from utoipa.
///
/// The method call is deliberately made at the handler's concrete type, not through a generic
/// helper: stable Rust resolves the two implementations by autoref, choosing the `ToSchema` one
/// when it applies and the no-op one otherwise. A foreign type without utoipa support therefore
/// keeps compiling, and is still marked unverified.
pub struct ForeignSchema<T>(std::marker::PhantomData<T>);

impl<T> ForeignSchema<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ForeignSchema<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ForeignSchemaProbe {
    fn foreign_schema(self, components: &mut Components) -> Option<RefOr<Schema>>;
}

impl<T: utoipa::ToSchema> ForeignSchemaProbe for &ForeignSchema<T> {
    fn foreign_schema(self, components: &mut Components) -> Option<RefOr<Schema>> {
        let name = T::name().into_owned();
        let mut referenced = Vec::new();
        <T as utoipa::ToSchema>::schemas(&mut referenced);
        for (component, schema) in referenced {
            components.schemas.insert(component, schema);
        }
        components.schemas.insert(name.clone(), T::schema());
        Some(utoipa::openapi::Ref::from_schema_name(name).into())
    }
}

impl<T> ForeignSchemaProbe for &&ForeignSchema<T> {
    fn foreign_schema(self, _components: &mut Components) -> Option<RefOr<Schema>> {
        None
    }
}

/// Type-driven probe for a handler's extractor arguments.
///
/// `routes!` sees only the tokens in the signature. A parameter spelled `Json<T>` may be
/// `roundhouse::Json<T>`, `axum::Json<T>`, an alias of either, or a user type with the same name.
/// This probe resolves the *type*, not the identifier, using the same autoref-specialisation
/// pattern as [`ForeignSchema`]: the most specific applicable implementation wins, and the
/// fallback marks an extractor the document cannot infer.
pub struct ExtractProbe<T>(std::marker::PhantomData<T>);

impl<T> ExtractProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ExtractProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeExtractor {
    /// Contribute this extractor to the handler's documentation.
    ///
    /// `add_failures` is false when the extractor sits inside a `Result<_, E>`; the handler then
    /// documents `E`'s responses instead of the extractor's own rejection.
    fn probe_extractor(
        self,
        components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    );
}

// `roundhouse::Json` validates constraints and is the type the document can promise.
impl<T: ApiSchema + 'static> ProbeExtractor for &ExtractProbe<crate::extract::Json<T>> {
    fn probe_extractor(
        self,
        components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        docs.body = Some(json_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
        if add_failures {
            docs.failures.extend(json_failures());
        }
    }
}

impl<T: ApiSchema + 'static> ProbeExtractor for &ExtractProbe<crate::extract::Form<T>> {
    fn probe_extractor(
        self,
        components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        docs.body = Some(form_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
        if add_failures {
            docs.failures.extend(form_failures());
        }
    }
}

#[cfg(feature = "multipart")]
impl<T: crate::multipart::MultipartForm + 'static> ProbeExtractor
    for &ExtractProbe<crate::multipart::Multipart<T>>
{
    fn probe_extractor(
        self,
        components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        docs.body = Some(multipart_body::<T>(components));
        for field in T::fields() {
            if field.kind.is_file() && field.max_bytes.is_none() {
                docs.unbounded_file_fields.push(field.name);
            }
        }
        docs.body_type = Some(|| TypeId::of::<T>());
        if add_failures {
            docs.failures.extend(multipart_failures());
        }
    }
}

impl<T: PathNames + 'static> ProbeExtractor for &ExtractProbe<crate::extract::Path<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        docs.params.extend(path_params::<T>());
        docs.path_schema = Some(<T as PathNames>::scalar_schema);
        if add_failures {
            docs.failures.extend(path_failures());
        }
    }
}

impl<T: QueryParams> ProbeExtractor for &ExtractProbe<crate::extract::Query<T>> {
    fn probe_extractor(
        self,
        components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        docs.params.extend(stored_query_params_for::<T>(components));
        if add_failures {
            docs.failures.extend(query_failures());
        }
    }
}

impl<T: Clone + Send + Sync + 'static> ProbeExtractor
    for &ExtractProbe<crate::extract::Extension<T>>
{
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        if add_failures {
            docs.failures.extend(extension_failures());
        }
    }
}

impl ProbeExtractor for &ExtractProbe<bytes::Bytes> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
        docs.body = Some(binary_body());
    }
}

impl ProbeExtractor for &ExtractProbe<axum::body::Body> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
        docs.body = Some(binary_body());
    }
}

// axum's own JSON extractor is documented from utoipa when it can be, but it does not enforce
// roundhouse's constraints, so the operation is marked unverified rather than promising them.
impl<T> ProbeExtractor for &ExtractProbe<axum::Json<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
        docs.unverified = true;
    }
}

/// A second, narrower probe for a foreign JSON body that has a utoipa schema.
///
/// The `ToSchema` bound must be resolved at the handler's concrete argument type, not inside the
/// generic extractor probe, so this is a separate autoref chain: `&&ForeignBodyProbe<axum::Json<T>>`
/// applies when `T: ToSchema`, and the `&&&` implementation for everything else does nothing.
pub struct ForeignBodyProbe<T>(std::marker::PhantomData<T>);

impl<T> ForeignBodyProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ForeignBodyProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeForeignBody {
    fn probe_foreign_body(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: utoipa::ToSchema> ProbeForeignBody for &&ForeignBodyProbe<axum::Json<T>> {
    fn probe_foreign_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        use utoipa::ToSchema;
        let name = T::name().into_owned();
        let mut referenced = Vec::new();
        <T as ToSchema>::schemas(&mut referenced);
        for (component, schema) in referenced {
            components.schemas.insert(component, schema);
        }
        components.schemas.insert(name.clone(), T::schema());
        docs.body = Some(BodyDoc {
            content_type: "application/json".to_string(),
            schema: utoipa::openapi::Ref::from_schema_name(name).into(),
            encoding: BTreeMap::new(),
            required: true,
            description: None,
        });
    }
}

impl<T> ProbeForeignBody for &&&ForeignBodyProbe<T> {
    fn probe_foreign_body(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
}

// axum's other extractors are known and contribute nothing the document cannot already see.
impl<T> ProbeExtractor for &ExtractProbe<axum::extract::State<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        _docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
    }
}

impl<T> ProbeExtractor for &ExtractProbe<axum::extract::ConnectInfo<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        _docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
    }
}

macro_rules! no_op_extractor {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ProbeExtractor for &ExtractProbe<$ty> {
                fn probe_extractor(
                    self,
                    _components: &mut Components,
                    _docs: &mut HandlerDocs,
                    _add_failures: bool,
                ) {
                }
            }
        )*
    };
}

no_op_extractor!(
    axum::http::HeaderMap,
    axum::http::Method,
    axum::http::Uri,
    axum::extract::OriginalUri,
    axum::extract::MatchedPath,
    axum::extract::Request,
    axum::http::request::Parts,
);

impl<T> ProbeExtractor for &ExtractProbe<Option<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        _docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
    }
}

impl<T> ProbeExtractor for &ExtractProbe<axum::extract::Path<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        if add_failures {
            docs.failures.extend(path_failures());
        }
    }
}

impl<T> ProbeExtractor for &ExtractProbe<axum::extract::Query<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        if add_failures {
            docs.failures.extend(query_failures());
        }
    }
}

impl<T> ProbeExtractor for &ExtractProbe<axum::extract::Extension<T>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
        docs.unverified_extractor = true;
    }
}

#[cfg(feature = "multipart")]
impl ProbeExtractor for &ExtractProbe<axum::extract::Multipart> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
        docs.unverified = true;
    }
}

// Anything else is an extractor the macro cannot reason about. Mark it instead of guessing.
impl<T> ProbeExtractor for &&ExtractProbe<T> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        _add_failures: bool,
    ) {
        docs.unverified_extractor = true;
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

pub fn canonical_reason(status: u16) -> &'static str {
    if status == 0 {
        return "Default Response";
    }
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
        encoding: BTreeMap::new(),
        required: true,
        description: None,
    }
}

#[cfg(feature = "multipart")]
pub fn multipart_body<F: crate::multipart::MultipartForm>(components: &mut Components) -> BodyDoc {
    let encoding = F::fields()
        .iter()
        .filter(|field| field.kind.is_file() && !field.accept.is_empty())
        .map(|field| {
            let content_type = field.accept.join(", ");
            (
                field.name.to_string(),
                utoipa::openapi::encoding::EncodingBuilder::new()
                    .content_type(Some(content_type))
                    .build(),
            )
        })
        .collect();
    BodyDoc {
        content_type: "multipart/form-data".to_string(),
        schema: F::schema(components),
        encoding,
        required: true,
        description: None,
    }
}

#[cfg(feature = "multipart")]
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
        encoding: BTreeMap::new(),
        required: true,
        description: None,
    }
}

/// A raw `Bytes` or `Body` request. The content type is the contract, not a schema.
pub fn binary_body() -> BodyDoc {
    BodyDoc {
        content_type: "application/octet-stream".to_string(),
        schema: schema::string_with_format(utoipa::openapi::schema::SchemaFormat::KnownFormat(
            utoipa::openapi::schema::KnownFormat::Binary,
        )),
        encoding: BTreeMap::new(),
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

impl<T: QueryParams> QueryParams for Option<T> {
    fn params(components: &mut Components) -> Vec<Param> {
        let mut params = T::params(components);
        for param in &mut params {
            param.required = false;
        }
        params
    }

    fn validate_at(&self, pointer: &str, errors: &mut crate::validate::ValidationErrors) {
        if let Some(value) = self {
            value.validate_at(pointer, errors);
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

/// One JSON response, for a `responses(200 = Post)` declaration.
pub fn json_response_doc<T: ApiSchema>(components: &mut Components, status: u16) -> ResponseDoc {
    T::register(components);
    ResponseDoc::json(status, T::schema())
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

/// Implemented by `#[derive(ApiResponse)]` and `#[derive(ApiError)]`, and by [`Problem`].
///
/// [`Problem`]: crate::Problem
pub trait ApiResponse {
    fn responses(components: &mut Components) -> Vec<ResponseDoc>;
}

/// Error-shaped response enum. Automatic once the type is an error, `IntoResponse` and `ApiResponse`.
pub trait ApiError: std::error::Error + axum::response::IntoResponse + ApiResponse {}

impl<T> ApiError for T where T: std::error::Error + axum::response::IntoResponse + ApiResponse {}
