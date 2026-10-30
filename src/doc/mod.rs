//! The documentation half: schema derivation, parameter tables and response declarations.
//!
//! Most applications use this module through the derives re-exported at the crate root:
//! `#[derive(ToSchema)]` for schemas, optionally `#[derive(ApiSchema)]` for named components,
//! roundhouse-only keywords and `validate_with` hooks, plus `#[derive(QueryParams)]`,
//! `#[derive(PathParams)]`, `#[derive(ApiResponse)]` and `#[derive(ApiError)]` (and
//! `#[derive(Multipart)]` with the `multipart` feature).
//!
//! The traits here are the manual escape hatch for types the derives cannot see:
//!
//! - [`ApiSchema`] — a named component plus roundhouse-only patches and runtime hooks; optional
//!   for ordinary `ToSchema` bodies, which are documented inline.
//! - [`ApiResponse`] — the documented responses a return type produces; `ApiError` is re-exported
//!   from the macros.
//! - [`ResponseDoc`] and [`layer`] — hand-written responses and documented layer guarantees.
//! - [`validate_document`] — the structural check every build runs over the finished document.
//!
//! Schemas are derived by `utoipa::ToSchema`. `#[derive(ApiSchema)]` is the optional extension for
//! named components, roundhouse-only keywords and `validate_with`. Component names follow utoipa:
//! unique names stay simple, and a collision is disambiguated with `#[schema(as = module::Type)]`.

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

pub use roundhouse_schema::{
    ApiSchema, Param, ParamIn, PartsSchema, PartsValue, PathNames, PathValidate,
};
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

/// How a handler reads the request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    Json,
    Form,
    Multipart,
    Raw,
}

/// The body a handler takes, as metadata for the request validator.
#[derive(Debug, Clone, Copy)]
pub struct BodyMeta {
    pub kind: BodyKind,
    pub type_id: fn() -> TypeId,
}

/// A path or query type whose parameter schema the engine should compile.
#[derive(Clone, Copy)]
pub struct PartsDoc {
    pub type_id: fn() -> TypeId,
    pub schema: fn(&mut Components) -> RefOr<Schema>,
}

impl PartsDoc {
    pub fn new<T: PartsSchema + 'static>() -> Self {
        Self {
            type_id: || TypeId::of::<T>(),
            schema: <T as PartsSchema>::parts_schema,
        }
    }
}

/// The documentation half a `#[handler]` attribute derives from the signature.
#[derive(Clone, Default)]
pub struct HandlerDocs {
    pub params: Vec<Param>,
    /// Path and query types the engine should compile for this operation.
    pub parts: Vec<PartsDoc>,
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
    /// A module-level guard for the same scheme is skipped: the extractor already performs the
    /// check during extraction, so the request must not be authenticated twice.
    const REQUIRED_AUTH: &'static [&'static str] = &[];

    /// The success statuses the return type guarantees, or `None` when the macro cannot classify
    /// it (a custom `ApiResponse`). `Some(&[])` means the macro knows there is no success
    /// response, so a route entry must declare one.
    const INFERRED_SUCCESS: Option<&'static [u16]> = None;

    /// The body the handler takes, if it has one. The router installs the raw JSON validator from
    /// this; `Result<Extractor, E>` bodies own their rejection and report `None`.
    fn body() -> Option<BodyMeta> {
        None
    }

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

/// A concrete-type probe for schema shape and component registration.
///
/// The specific impl uses [`ApiSchema`] (and therefore its roundhouse patches/components); the
/// fallback uses plain `utoipa::ToSchema` and registers only referenced components. Call this at
/// the concrete type: a generic wrapper would lose the bound and always choose the fallback.
pub struct SchemaFor<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> SchemaFor<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for SchemaFor<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeSchemaFor {
    fn probe_schema(self) -> RefOr<Schema>;
    fn probe_register(self, components: &mut Components);
}

impl<T: ApiSchema + ?Sized> ProbeSchemaFor for &SchemaFor<T> {
    fn probe_schema(self) -> RefOr<Schema> {
        T::schema()
    }

    fn probe_register(self, components: &mut Components) {
        T::register(components);
    }
}

impl<T: utoipa::ToSchema + ?Sized> ProbeSchemaFor for &&SchemaFor<T> {
    fn probe_schema(self) -> RefOr<Schema> {
        <T as utoipa::PartialSchema>::schema()
    }

    fn probe_register(self, components: &mut Components) {
        register_self::<T>(components);
    }
}

/// A concrete-type probe for recursive `validate_with` traversal.
///
/// The specific impl calls the field type's [`ApiSchema::validate_at`]; the fallback is a no-op
/// for a plain `ToSchema` field. Call this at the concrete field type.
pub struct ValidateFor<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> ValidateFor<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for ValidateFor<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeValidateFor<T: ?Sized> {
    fn probe_validate_for(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    );
}

impl<T: ApiSchema + ?Sized> ProbeValidateFor<T> for &ValidateFor<T> {
    fn probe_validate_for(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    ) {
        value.validate_at(pointer, errors);
    }
}

impl<T: utoipa::ToSchema + ?Sized> ProbeValidateFor<T> for &&ValidateFor<T> {
    fn probe_validate_for(
        self,
        _value: &T,
        _pointer: &str,
        _errors: &mut crate::validate::ValidationErrors,
    ) {
    }
}

/// A concrete-type probe for a JSON response body.
pub struct JsonResponseProbe<T>(std::marker::PhantomData<T>);

impl<T> JsonResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for JsonResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeJsonResponse {
    fn probe_json_response(self, components: &mut Components, status: u16) -> Vec<ResponseDoc>;
}

impl<T: ApiSchema> ProbeJsonResponse for &JsonResponseProbe<T> {
    fn probe_json_response(self, components: &mut Components, status: u16) -> Vec<ResponseDoc> {
        json_response::<T>(components, status)
    }
}

impl<T> ProbeJsonResponse for &&JsonResponseProbe<T> {
    fn probe_json_response(self, _components: &mut Components, _status: u16) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

/// A second response probe for types that derive only `ToSchema`.
pub struct ToSchemaJsonResponseProbe<T>(std::marker::PhantomData<T>);

impl<T> ToSchemaJsonResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ToSchemaJsonResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeToSchemaJsonResponse {
    fn probe_to_schema_json_response(
        self,
        components: &mut Components,
        status: u16,
    ) -> Vec<ResponseDoc>;
}

impl<T: utoipa::ToSchema> ProbeToSchemaJsonResponse for &ToSchemaJsonResponseProbe<T> {
    fn probe_to_schema_json_response(
        self,
        components: &mut Components,
        status: u16,
    ) -> Vec<ResponseDoc> {
        json_response_to_schema::<T>(components, status)
    }
}

impl<T> ProbeToSchemaJsonResponse for &&ToSchemaJsonResponseProbe<T> {
    fn probe_to_schema_json_response(
        self,
        _components: &mut Components,
        _status: u16,
    ) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

/// A concrete-type probe for a `responses(status = Type)` declaration.
pub struct JsonResponseDocProbe<T>(std::marker::PhantomData<T>);

impl<T> JsonResponseDocProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for JsonResponseDocProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeJsonResponseDoc {
    fn probe_json_response_doc(self, components: &mut Components, status: u16) -> ResponseDoc;
}

impl<T: ApiSchema> ProbeJsonResponseDoc for &JsonResponseDocProbe<T> {
    fn probe_json_response_doc(self, components: &mut Components, status: u16) -> ResponseDoc {
        json_response_doc::<T>(components, status)
    }
}

impl<T: utoipa::ToSchema> ProbeJsonResponseDoc for &&JsonResponseDocProbe<T> {
    fn probe_json_response_doc(self, components: &mut Components, status: u16) -> ResponseDoc {
        json_response_doc_to_schema::<T>(components, status)
    }
}

/// A concrete-type probe for a non-JSON response body.
pub struct TextResponseProbe<T>(std::marker::PhantomData<T>);

impl<T> TextResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for TextResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeTextResponse {
    fn probe_text_response(
        self,
        components: &mut Components,
        status: u16,
        content_type: &str,
    ) -> ResponseDoc;
}

impl<T: ApiSchema> ProbeTextResponse for &TextResponseProbe<T> {
    fn probe_text_response(
        self,
        components: &mut Components,
        status: u16,
        content_type: &str,
    ) -> ResponseDoc {
        text_response::<T>(components, status, content_type)
    }
}

impl<T: utoipa::ToSchema> ProbeTextResponse for &&TextResponseProbe<T> {
    fn probe_text_response(
        self,
        components: &mut Components,
        status: u16,
        content_type: &str,
    ) -> ResponseDoc {
        text_response_to_schema::<T>(components, status, content_type)
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

impl<T: QueryParams> ProbeExtractor for &ExtractProbe<crate::extract::RepeatedQuery<T>> {
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

/// A probe for a JSON body whose type has an `ApiSchema` (including composite bodies such as
/// `Vec<T>` that have no component of their own).
pub struct JsonBodyProbe<T>(std::marker::PhantomData<T>);

impl<T> JsonBodyProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for JsonBodyProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeJsonBody {
    fn probe_json_body(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: ApiSchema + 'static> ProbeJsonBody for &&JsonBodyProbe<axum::Json<T>> {
    fn probe_json_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(json_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
    }
}

impl<T: ApiSchema + 'static> ProbeJsonBody
    for &&JsonBodyProbe<crate::extract::Strict<axum::Json<T>>>
{
    fn probe_json_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(json_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
    }
}

impl<T> ProbeJsonBody for &&&JsonBodyProbe<T> {
    fn probe_json_body(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
}

/// A second body probe for types that derive only `ToSchema`.
///
/// Kept separate from `JsonBodyProbe` so the two autoref chains do not overlap: the `ApiSchema`
/// probe is exact at `&&`, this one is exact at `&&` for its own probe type, and each has its own
/// `&&&` no-op fallback. The handler calls both; only one sets `docs.body`.
pub struct ToSchemaBodyProbe<T>(std::marker::PhantomData<T>);

impl<T> ToSchemaBodyProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ToSchemaBodyProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeToSchemaBody {
    fn probe_to_schema_body(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: utoipa::ToSchema + 'static> ProbeToSchemaBody for &&ToSchemaBodyProbe<axum::Json<T>> {
    fn probe_to_schema_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        if docs.body.is_none() {
            docs.body = Some(json_body_to_schema::<T>(components));
            docs.body_type = Some(|| TypeId::of::<T>());
        }
    }
}

impl<T: utoipa::ToSchema + 'static> ProbeToSchemaBody
    for &&ToSchemaBodyProbe<crate::extract::Strict<axum::Json<T>>>
{
    fn probe_to_schema_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        if docs.body.is_none() {
            docs.body = Some(json_body_to_schema::<T>(components));
            docs.body_type = Some(|| TypeId::of::<T>());
        }
    }
}

impl<T> ProbeToSchemaBody for &&&ToSchemaBodyProbe<T> {
    fn probe_to_schema_body(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
}

/// A probe for the path/query types an operation uses.
///
/// The engine compiles the schema for each parts type once. The handler macro invokes this at the
/// concrete argument type, so a scalar `Path<T>` without a `PartsSchema` impl simply contributes
/// nothing.
pub struct PartsProbe<T>(std::marker::PhantomData<T>);

impl<T> PartsProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for PartsProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeParts {
    fn probe_parts(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: PartsSchema + 'static> ProbeParts for &&PartsProbe<axum::extract::Path<T>> {
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts for &&PartsProbe<crate::extract::RepeatedQuery<T>> {
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts for &&PartsProbe<axum::extract::Query<T>> {
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts
    for &&PartsProbe<crate::extract::Strict<axum::extract::Path<T>>>
{
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts
    for &&PartsProbe<crate::extract::Strict<axum::extract::Query<T>>>
{
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts
    for &&PartsProbe<crate::extract::Strict<crate::extract::RepeatedQuery<T>>>
{
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T> ProbeParts for &&&PartsProbe<T> {
    fn probe_parts(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
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
        // `ApiSchema::register` already inserted this type (with its schema patches, such as
        // `url` → `uri` and `uniqueItems`). Do not overwrite it with the raw `ToSchema` output.
        if !components.schemas.contains_key(&name) {
            let mut referenced = Vec::new();
            <T as ToSchema>::schemas(&mut referenced);
            for (component, schema) in referenced {
                components.schemas.insert(component, schema);
            }
            components.schemas.insert(name.clone(), T::schema());
        }
        if docs.body.is_none() {
            docs.body = Some(BodyDoc {
                content_type: "application/json".to_string(),
                schema: utoipa::openapi::Ref::from_schema_name(name).into(),
                encoding: BTreeMap::new(),
                required: true,
                description: None,
            });
        }
    }
}

impl<T: utoipa::ToSchema> ProbeForeignBody
    for &&ForeignBodyProbe<crate::extract::Strict<axum::Json<T>>>
{
    fn probe_foreign_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        (&&ForeignBodyProbe::<axum::Json<T>>::new()).probe_foreign_body(components, docs);
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

impl<T: PathNames + 'static> ProbeExtractor for &ExtractProbe<axum::extract::Path<T>> {
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

impl<T: QueryParams> ProbeExtractor for &ExtractProbe<axum::extract::Query<T>> {
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
impl<T> ProbeExtractor for &ExtractProbe<crate::extract::Strict<axum::Json<T>>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        // The body itself is registered by `ForeignBodyProbe` when `T: ToSchema`.
        docs.unverified = true;
        if add_failures {
            docs.failures.extend(json_failures());
        }
    }
}

impl<T: ApiSchema + 'static> ProbeExtractor
    for &ExtractProbe<crate::extract::Strict<axum::Form<T>>>
{
    fn probe_extractor(
        self,
        components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        docs.body = Some(form_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
        docs.unverified = true;
        if add_failures {
            docs.failures.extend(form_failures());
        }
    }
}

impl<T: PathNames + 'static> ProbeExtractor
    for &ExtractProbe<crate::extract::Strict<axum::extract::Path<T>>>
{
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

impl<T: QueryParams> ProbeExtractor
    for &ExtractProbe<crate::extract::Strict<axum::extract::Query<T>>>
{
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

impl<T> ProbeExtractor for &ExtractProbe<crate::extract::Strict<axum::extract::Extension<T>>> {
    fn probe_extractor(
        self,
        _components: &mut Components,
        docs: &mut HandlerDocs,
        add_failures: bool,
    ) {
        docs.unverified_extractor = true;
        if add_failures {
            docs.failures.extend(extension_failures());
        }
    }
}

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

/// Register the components referenced by a plain `ToSchema` type.
pub fn register_to_schema<T: utoipa::ToSchema + ?Sized>(components: &mut Components) {
    let mut referenced = Vec::new();
    <T as utoipa::ToSchema>::schemas(&mut referenced);
    for (name, schema) in referenced {
        components.schemas.entry(name).or_insert(schema);
    }
}

/// Register just this type as a component, without walking its `schemas()`.
///
/// `ToSchema::schemas()` recurses for self-referential types; registration driven by an enclosing
/// `ApiSchema` derive calls this per field instead, so a recursive field cannot overflow.
pub fn register_self<T: utoipa::ToSchema + ?Sized>(components: &mut Components) {
    let name = <T as utoipa::ToSchema>::name().into_owned();
    // Composite types have no stable component identity (`Vec`, `Option`, ...). Their item
    // schemas are inlined or registered by the concrete item type; never invent a component.
    if matches!(
        name.as_str(),
        "Vec"
            | "Option"
            | "Box"
            | "Arc"
            | "Rc"
            | "Cow"
            | "HashMap"
            | "BTreeMap"
            | "HashSet"
            | "BTreeSet"
            | "LinkedList"
            | "IndexMap"
            | "IndexSet"
            | "String"
            | "str"
            | "Value"
    ) {
        return;
    }
    if components.schemas.contains_key(&name) {
        return;
    }
    components.schemas.insert(name.clone(), schema::any_value());
    components
        .schemas
        .insert(name, <T as utoipa::PartialSchema>::schema());
}

/// A JSON body documented from `ToSchema` alone, without roundhouse hooks.
pub fn json_body_to_schema<T: utoipa::ToSchema>(components: &mut Components) -> BodyDoc {
    register_to_schema::<T>(components);
    BodyDoc {
        content_type: "application/json".to_string(),
        schema: <T as utoipa::PartialSchema>::schema(),
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

pub fn path_params<T: PathNames>() -> Vec<Param> {
    T::params()
}

/// Query parameters declared by `#[derive(QueryParams)]`.
pub trait QueryParams {
    fn params(components: &mut Components) -> Vec<Param>;

    /// Run the custom `validate_with` hooks; declarative constraints come from the parameter
    /// schema.
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

pub fn stored_query_params_for<T: QueryParams + ?Sized>(components: &mut Components) -> Vec<Param> {
    <T as QueryParams>::params(components)
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
