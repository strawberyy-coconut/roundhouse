use std::any::TypeId;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, Schema};

use super::super::{
    ApiSchema, HandlerDocs, PathNames, QueryParams, binary_body, extension_failures, form_body,
    form_failures, json_failures, multipart_body, multipart_failures, path_failures, path_params,
    query_failures, stored_query_params_for,
};

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
