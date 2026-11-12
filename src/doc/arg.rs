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

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

use crate::auth::{Auth, AuthDoc, AuthScheme, SchemeDoc, auth_doc, auth_scheme};
use crate::doc::body::{binary_body, form_body, json_body};
#[cfg(feature = "multipart")]
use crate::doc::body::{multipart_body, multipart_failures};
use crate::doc::response_doc::{
    auth_failures, extension_failures, form_failures, json_failures, path_failures, query_failures,
};
use crate::doc::{
    ApiSchema, BodyDoc, DocContext, OperationDoc, Param, Params, PartsDoc, path_params,
    stored_query_params_for,
};
use crate::extract::{Mapped, RepeatedQuery, Strict, ValidationContext};
use crate::problem::Problem;

/// What one handler argument contributes to the operation.
///
/// `ExtractArg::document` returns this value; the document builder owns the merge rules. A trait
/// never mutates the shared `OperationDoc` bag.
#[derive(Default)]
pub struct ArgDoc {
    pub params: Vec<Param>,
    pub body: Option<BodyDoc>,
    pub body_type: Option<fn() -> TypeId>,
    pub parts: Vec<PartsDoc>,
    pub failures: Vec<crate::doc::ResponseDoc>,
    pub required_auth: Vec<AuthDoc>,
    pub optional_auth: Vec<AuthDoc>,
    pub schemes: Vec<SchemeDoc>,
    pub path_schema: Option<fn() -> Option<RefOr<Schema>>>,
    pub unverified_body: bool,
    pub unverified_extractor: bool,
    pub unbounded_file_fields: Vec<&'static str>,
}

impl ArgDoc {
    /// `Option<T>` publications: every parameter and the body become optional, and required auth
    /// moves to the optional alternative.
    pub fn optional(mut self) -> Self {
        for param in &mut self.params {
            param.required = false;
        }
        if let Some(body) = &mut self.body {
            body.required = false;
        }
        let had_required_auth = !self.required_auth.is_empty();
        self.optional_auth.append(&mut self.required_auth);
        if had_required_auth {
            // `Option<Auth<A>>` documents the scheme as an alternative and does not promise the
            // required-auth rejection responses.
            self.failures.clear();
        }
        self
    }

    /// `Result<T, E>` publications: the handler owns the rejection, so no extractor failures are
    /// promised and the shim must not validate the body a second time.
    pub fn result(mut self) -> Self {
        self.failures.clear();
        self.body_type = None;
        self
    }
}

impl OperationDoc {
    /// Merge one argument's contribution into the operation being documented.
    pub fn merge(&mut self, arg: ArgDoc) {
        self.params.extend(arg.params);
        if arg.body.is_some() {
            self.body = arg.body;
        }
        if arg.body_type.is_some() {
            self.body_type = arg.body_type;
        }
        self.parts.extend(arg.parts);
        self.failures.extend(arg.failures);
        self.required_auth.extend(arg.required_auth);
        self.optional_auth.extend(arg.optional_auth);
        self.schemes.extend(arg.schemes);
        if arg.path_schema.is_some() {
            self.path_schema = arg.path_schema;
        }
        self.unverified |= arg.unverified_body;
        self.unverified_extractor |= arg.unverified_extractor;
        self.unbounded_file_fields.extend(arg.unbounded_file_fields);
    }
}

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

    /// The argument's documentation. Associated, because the macro describes the extractor type
    /// before a value exists.
    fn document(context: &mut DocContext<'_>) -> ArgDoc;

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
    T: Params + 'static,
{
    crate::extract::enforce_parts_schema(value, context.engine(), context.status())
}

// --- Bodies -------------------------------------------------------------------------------

impl<T: ApiSchema + 'static> ExtractArg for axum::Json<T> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(json_body::<T>(context)),
            body_type: Some(|| TypeId::of::<T>()),
            unverified_body: true,
            failures: json_failures(),
            ..ArgDoc::default()
        }
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

    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(form_body::<T>(context)),
            body_type: Some(|| TypeId::of::<T>()),
            unverified_body: true,
            failures: form_failures(),
            ..ArgDoc::default()
        }
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_body(&self.0, context)
    }
}

impl ExtractArg for bytes::Bytes {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(binary_body()),
            ..ArgDoc::default()
        }
    }
}

impl ExtractArg for axum::body::Body {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(binary_body()),
            ..ArgDoc::default()
        }
    }
}

// A plain `String` body is a raw request body the framework cannot describe; keep the current
// honesty marker rather than guessing at a media type.
impl ExtractArg for String {
    const IS_BODY: bool = true;

    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_extractor: true,
            ..ArgDoc::default()
        }
    }
}

// A `Request` body has already consumed the request; it contributes no schema.
impl ExtractArg for axum::extract::Request {
    const IS_BODY: bool = true;

    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc::default()
    }
}

#[cfg(feature = "multipart")]
impl<F: crate::multipart::MultipartForm + 'static> ExtractArg for crate::multipart::Multipart<F> {
    const IS_BODY: bool = true;

    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        let mut doc = ArgDoc {
            body: Some(multipart_body::<F>(context)),
            body_type: Some(|| TypeId::of::<F>()),
            failures: multipart_failures(),
            ..ArgDoc::default()
        };
        for field in F::fields() {
            if field.kind.is_file() && field.max_bytes.is_none() {
                doc.unbounded_file_fields.push(field.name);
            }
        }
        doc
    }
}

#[cfg(feature = "multipart")]
impl ExtractArg for axum::extract::Multipart {
    const IS_BODY: bool = true;

    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_body: true,
            ..ArgDoc::default()
        }
    }
}

impl<F, M> ExtractArg for Result<F, M>
where
    F: ExtractArg,
{
    const IS_BODY: bool = F::IS_BODY;

    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        // A `Result<Extractor, E>` owns its rejection. The handler receives the success value or
        // its own error type, so the route shim (and the raw body layer) must not enforce the
        // schema a second time. Keep the documented shape, drop the runtime registration.
        F::document(context).result()
    }

    // The handler owns the rejection (for example `Result<Json<T>, JsonRejection>`), so the shim
    // does not validate it again.
}

// --- Parts --------------------------------------------------------------------------------

impl<T: Params + 'static> ExtractArg for axum::extract::Path<T> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        T::register(context);
        let mut doc = ArgDoc {
            params: path_params::<T>(context),
            path_schema: Some(T::scalar_schema),
            failures: path_failures(),
            ..ArgDoc::default()
        };
        if let Some(parts) = T::parts_doc() {
            doc.parts.push(parts);
        }
        doc
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)?;
        Params::validate(&self.0).map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: Params + 'static> ExtractArg for axum::extract::Query<T> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        let mut doc = ArgDoc {
            params: stored_query_params_for::<T>(context),
            failures: query_failures(),
            ..ArgDoc::default()
        };
        if let Some(parts) = T::parts_doc() {
            doc.parts.push(parts);
        }
        doc
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)?;
        Params::validate(&self.0).map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: Params + 'static> ExtractArg for RepeatedQuery<T> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        let mut doc = ArgDoc {
            params: stored_query_params_for::<T>(context),
            failures: query_failures(),
            ..ArgDoc::default()
        };
        if let Some(parts) = T::parts_doc() {
            doc.parts.push(parts);
        }
        doc
    }

    // The extractor itself validates the query before returning it.
}

impl<T> ExtractArg for axum::extract::Extension<T> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_extractor: true,
            ..ArgDoc::default()
        }
    }
}

impl<S> ExtractArg for axum::extract::State<S> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc::default()
    }
}

#[cfg(feature = "serve")]
impl<T> ExtractArg for axum::extract::ConnectInfo<T> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc::default()
    }
}

macro_rules! no_op_extractor {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ExtractArg for $ty {
                fn document(_context: &mut DocContext<'_>) -> ArgDoc {
                    ArgDoc::default()
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
    axum::http::request::Parts,
);

// --- Auth ---------------------------------------------------------------------------------

impl<A, S> ExtractArg for Auth<A, S>
where
    A: AuthScheme,
    S: crate::auth::Scopes<A>,
{
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            schemes: vec![auth_scheme::<A>()],
            required_auth: vec![auth_doc::<A, S>()],
            failures: auth_failures::<A>(),
            ..ArgDoc::default()
        }
    }
}

// --- Wrappers -----------------------------------------------------------------------------

impl<T: ExtractArg> ExtractArg for Option<T> {
    const IS_BODY: bool = T::IS_BODY;
    const SHIM_VALIDATES: bool = T::SHIM_VALIDATES;

    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        T::document(context).optional()
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        match self {
            Some(value) => value.validate(context),
            None => Ok(()),
        }
    }
}

impl<T> ExtractArg for Mapped<T> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        // `Mapped` only maps a rejection; it deliberately does not document or enforce a schema.
        ArgDoc {
            unverified_extractor: true,
            ..ArgDoc::default()
        }
    }
}

impl<T: ApiSchema + 'static> ExtractArg for Strict<axum::Json<T>> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::Json<T> as ExtractArg>::document(context)
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

    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::Form<T> as ExtractArg>::document(context)
    }

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        self.0
            .0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: Params + 'static> ExtractArg for Strict<axum::extract::Path<T>> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::extract::Path<T> as ExtractArg>::document(context)
    }

    // `Strict` already enforced the parts schema during extraction.
}

impl<T: Params + 'static> ExtractArg for Strict<axum::extract::Query<T>> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::extract::Query<T> as ExtractArg>::document(context)
    }
}

impl<T: Params + 'static> ExtractArg for Strict<RepeatedQuery<T>> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <RepeatedQuery<T> as ExtractArg>::document(context)
    }
}

impl<T> ExtractArg for Strict<axum::extract::Extension<T>> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_extractor: true,
            failures: extension_failures(),
            ..ArgDoc::default()
        }
    }
}

#[cfg(feature = "multipart")]
impl<F: crate::multipart::MultipartForm + 'static> ExtractArg
    for Strict<crate::multipart::Multipart<F>>
{
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <crate::multipart::Multipart<F> as ExtractArg>::document(context)
    }
}
