//! Extractor documentation and route-level validation.
//!
//! `#[handler]` no longer classifies argument types. It asks each axum extractor how it
//! contributes to the document through [`ExtractArg`], and how it should be validated through
//! [`ExtractArg::validate`]. Adding support for a custom extractor is therefore one impl in the
//! crate that owns the type.
//!
//! Automatic body documentation requires [`ApiSchema`]: it is the trait that publishes the
//! patched component (`unique_items`, `url` → `uri`, `validate_with`) the document promises. A
//! type that only implements `ToSchema` can still be used in an explicit
//! `@responses: status: Type` declaration.

use std::any::TypeId;

use utoipa::openapi::schema::Components;

use crate::auth::{Auth, AuthScheme, AuthScopes, auth_doc, auth_scheme};
use crate::doc::body::{binary_body, form_body, json_body};
#[cfg(feature = "multipart")]
use crate::doc::body::{multipart_body, multipart_failures};
use crate::doc::response_doc::{
    auth_failures, extension_failures, form_failures, json_failures, path_failures, query_failures,
};
use crate::doc::{
    ApiSchema, HandlerDocs, PartsValue, PathNames, PathValidate, QueryParams, path_params,
    stored_query_params_for,
};
use crate::extract::{Mapped, RepeatedQuery, Strict, ValidationContext};
use crate::problem::Problem;

/// One route argument's contribution to the document and to route-level validation.
///
/// Implement this for an extractor type to make it first-class in a documented route. The default
/// `validate` is a no-op, which is right for extractors that enforce their own contract (or have
/// no request constraints).
pub trait ExtractArg {
    /// Whether this extractor consumes the request body. The handler macro uses this only to keep
    /// the validation marker honest; axum still enforces the one-body rule and the argument order.
    const IS_BODY: bool = false;

    /// Whether the route shim re-checks this body after extraction. Body extractors that already
    /// executed the schema during extraction (`Strict`, multipart) set this to `false`.
    const SHIM_VALIDATES: bool = false;

    fn document(components: &mut Components, docs: &mut HandlerDocs);

    /// Documentation for `Option<Self>`. Optional auth, query and body arguments override this to
    /// publish their alternative shape.
    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        Self::document(components, docs);
    }

    /// Documentation when the extractor sits inside `Result<Self, E>`.
    ///
    /// The handler owns the rejection, so extractor-specific failure responses are not promised.
    /// The successful shape (body, parameters, auth) is still documented.
    fn document_result(components: &mut Components, docs: &mut HandlerDocs) {
        Self::document(components, docs);
    }

    /// Route-level validation after extraction. The default leaves the value alone.
    #[allow(clippy::result_large_err)]
    fn validate(&self, _context: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }
}

#[allow(clippy::result_large_err)]
fn validation_problem(
    errors: crate::validate::ValidationErrors,
    context: &ValidationContext,
) -> Problem {
    crate::extract::validation_problem(errors, context.status())
}

#[allow(clippy::result_large_err)]
fn validate_body<T>(value: &T, context: &ValidationContext) -> Result<(), Problem>
where
    T: ApiSchema + serde::Serialize + 'static,
{
    crate::extract::enforce_schema(value, context.engine(), context.status())?;
    value
        .validate()
        .map_err(|errors| validation_problem(errors, context))
}

#[allow(clippy::result_large_err)]
fn validate_parts<T>(value: &T, context: &ValidationContext) -> Result<(), Problem>
where
    T: PartsValue + 'static,
{
    crate::extract::enforce_parts_schema(value, context.engine(), context.status())
}

// --- Bodies -------------------------------------------------------------------------------

impl<T: ApiSchema + 'static> ExtractArg for axum::Json<T> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(json_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
        docs.unverified = true;
        docs.failures.extend(json_failures());
    }

    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        Self::document(components, docs);
        if let Some(body) = &mut docs.body {
            body.required = false;
        }
    }

    fn document_result(components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(json_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
        docs.unverified = true;
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        // The raw JSON layer already executed the declared schema; only the custom hooks remain.
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: ApiSchema + serde::Serialize + 'static> ExtractArg for axum::Form<T> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(form_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
        docs.unverified = true;
        docs.failures.extend(form_failures());
    }

    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        Self::document(components, docs);
        if let Some(body) = &mut docs.body {
            body.required = false;
        }
    }

    fn document_result(components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(form_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
        docs.unverified = true;
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_body(&self.0, context)
    }
}

impl ExtractArg for bytes::Bytes {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(binary_body());
    }
}

impl ExtractArg for axum::body::Body {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(binary_body());
    }
}

// A plain `String` body is a raw request body the framework cannot describe; keep the current
// honesty marker rather than guessing at a media type.
impl ExtractArg for String {
    const IS_BODY: bool = true;

    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.unverified_extractor = true;
    }
}

// A `Request` body has already consumed the request; it contributes no schema.
impl ExtractArg for axum::extract::Request {
    const IS_BODY: bool = true;

    fn document(_components: &mut Components, _docs: &mut HandlerDocs) {}
}

#[cfg(feature = "multipart")]
impl<F: crate::multipart::MultipartForm + 'static> ExtractArg for crate::multipart::Multipart<F> {
    const IS_BODY: bool = true;

    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(multipart_body::<F>(components));
        for field in F::fields() {
            if field.kind.is_file() && field.max_bytes.is_none() {
                docs.unbounded_file_fields.push(field.name);
            }
        }
        docs.body_type = Some(|| TypeId::of::<F>());
        docs.failures.extend(multipart_failures());
    }
}

#[cfg(feature = "multipart")]
impl ExtractArg for axum::extract::Multipart {
    const IS_BODY: bool = true;

    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.unverified = true;
    }
}

impl<F, M> ExtractArg for Result<F, M>
where
    F: ExtractArg,
{
    const IS_BODY: bool = F::IS_BODY;

    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        F::document_result(components, docs);
        // A `Result<Extractor, E>` owns its rejection. The handler receives the success value or
        // its own error type, so the route shim (and the raw body layer) must not enforce the
        // schema a second time. Keep the documented shape, drop the runtime registration.
        docs.body_type = None;
    }

    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        F::document_result(components, docs);
        docs.body_type = None;
    }

    // The handler owns the rejection (for example `Result<Json<T>, JsonRejection>`), so the shim
    // does not validate it again.
}

// --- Parts --------------------------------------------------------------------------------

impl<T: PathNames + PartsValue + PathValidate + 'static> ExtractArg for axum::extract::Path<T> {
    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.params.extend(path_params::<T>());
        docs.path_schema = Some(<T as PathNames>::scalar_schema);
        if let Some(parts) = T::parts_doc() {
            docs.parts.push(parts);
        }
        docs.failures.extend(path_failures());
    }

    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        let start = docs.params.len();
        Self::document(components, docs);
        for param in &mut docs.params[start..] {
            param.required = false;
        }
    }

    fn document_result(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.params.extend(path_params::<T>());
        docs.path_schema = Some(<T as PathNames>::scalar_schema);
        if let Some(parts) = T::parts_doc() {
            docs.parts.push(parts);
        }
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)?;
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: QueryParams + PartsValue + 'static> ExtractArg for axum::extract::Query<T> {
    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        docs.params.extend(stored_query_params_for::<T>(components));
        if let Some(parts) = T::parts_doc() {
            docs.parts.push(parts);
        }
        docs.failures.extend(query_failures());
    }

    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        let start = docs.params.len();
        Self::document(components, docs);
        for param in &mut docs.params[start..] {
            param.required = false;
        }
    }

    fn document_result(components: &mut Components, docs: &mut HandlerDocs) {
        docs.params.extend(stored_query_params_for::<T>(components));
        if let Some(parts) = T::parts_doc() {
            docs.parts.push(parts);
        }
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)?;
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: QueryParams + PartsValue + 'static> ExtractArg for RepeatedQuery<T> {
    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        docs.params.extend(stored_query_params_for::<T>(components));
        if let Some(parts) = T::parts_doc() {
            docs.parts.push(parts);
        }
        docs.failures.extend(query_failures());
    }

    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        let start = docs.params.len();
        Self::document(components, docs);
        for param in &mut docs.params[start..] {
            param.required = false;
        }
    }

    fn document_result(components: &mut Components, docs: &mut HandlerDocs) {
        docs.params.extend(stored_query_params_for::<T>(components));
        if let Some(parts) = T::parts_doc() {
            docs.parts.push(parts);
        }
    }

    // The extractor itself validates the query before returning it.
}

impl<T> ExtractArg for axum::extract::Extension<T> {
    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.unverified_extractor = true;
    }
}

impl<S> ExtractArg for axum::extract::State<S> {
    fn document(_components: &mut Components, _docs: &mut HandlerDocs) {}
}

#[cfg(feature = "serve")]
impl<T> ExtractArg for axum::extract::ConnectInfo<T> {
    fn document(_components: &mut Components, _docs: &mut HandlerDocs) {}
}

macro_rules! no_op_extractor {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ExtractArg for $ty {
                fn document(_components: &mut Components, _docs: &mut HandlerDocs) {}
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
    axum::http::request::Parts,
);

// --- Auth ---------------------------------------------------------------------------------

impl<A, Scopes> ExtractArg for Auth<A, Scopes>
where
    A: AuthScheme,
    Scopes: AuthScopes,
{
    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.schemes.push(auth_scheme::<A>());
        docs.required_auth.push(auth_doc::<A, Scopes>());
        docs.failures.extend(auth_failures::<A>());
    }

    fn document_optional(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.schemes.push(auth_scheme::<A>());
        docs.optional_auth.push(auth_doc::<A, Scopes>());
    }
}

// --- Wrappers -----------------------------------------------------------------------------

impl<T: ExtractArg> ExtractArg for Option<T> {
    const IS_BODY: bool = T::IS_BODY;
    const SHIM_VALIDATES: bool = T::SHIM_VALIDATES;

    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        T::document_optional(components, docs);
    }

    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) {
        T::document_optional(components, docs);
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        match self {
            Some(value) => value.validate(context),
            None => Ok(()),
        }
    }
}

impl<T> ExtractArg for Mapped<T> {
    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        // `Mapped` only maps a rejection; it deliberately does not document or enforce a schema.
        docs.unverified_extractor = true;
    }
}

impl<T: ApiSchema + 'static> ExtractArg for Strict<axum::Json<T>> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        <axum::Json<T> as ExtractArg>::document(components, docs);
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        // `Strict` already executed the schema; run only the custom hooks.
        self.0
            .0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: ApiSchema + serde::Serialize + 'static> ExtractArg for Strict<axum::Form<T>> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        <axum::Form<T> as ExtractArg>::document(components, docs);
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        self.0
            .0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: PathNames + PartsValue + PathValidate + 'static> ExtractArg
    for Strict<axum::extract::Path<T>>
{
    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        <axum::extract::Path<T> as ExtractArg>::document(components, docs);
    }

    // `Strict` already enforced the parts schema during extraction.
}

impl<T: QueryParams + PartsValue + 'static> ExtractArg for Strict<axum::extract::Query<T>> {
    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        <axum::extract::Query<T> as ExtractArg>::document(components, docs);
    }
}

impl<T: QueryParams + PartsValue + 'static> ExtractArg for Strict<RepeatedQuery<T>> {
    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        <RepeatedQuery<T> as ExtractArg>::document(components, docs);
    }
}

impl<T> ExtractArg for Strict<axum::extract::Extension<T>> {
    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.unverified_extractor = true;
        docs.failures.extend(extension_failures());
    }
}

#[cfg(feature = "multipart")]
impl<F: crate::multipart::MultipartForm + 'static> ExtractArg
    for Strict<crate::multipart::Multipart<F>>
{
    fn document(components: &mut Components, docs: &mut HandlerDocs) {
        <crate::multipart::Multipart<F> as ExtractArg>::document(components, docs);
    }
}
