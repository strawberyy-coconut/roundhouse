//! Extractor documentation: what one handler argument contributes to the operation.
//!
//! `#[handler]` asks each axum extractor how it contributes to the document through
//! [`ExtractDoc`]. Route-level validation lives on the runtime `ExtractArg` trait, so adding a
//! custom extractor means one runtime impl and, when it has something to document, one
//! documentation impl.
//!
//! Automatic body documentation requires [`ApiSchema`]: it is the trait that publishes the
//! patched component (`unique_items`, `url` → `uri`, `validate_with`) the document promises. A
//! type that only implements `ToSchema` can still be used in an explicit
//! typed `response(status, Type)` declaration.

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

use crate::auth::{Auth, AuthDoc, AuthScheme, SchemeDoc, Scopes, auth_doc, auth_scheme};
use crate::doc::body::{binary_body, form_body, json_body};
#[cfg(feature = "multipart")]
use crate::doc::body::{multipart_body, multipart_failures};
use crate::doc::response_doc::{
    auth_failures, extension_failures, form_failures, json_failures, path_failures, query_failures,
};
use crate::doc::{
    ApiSchema, BodyDoc, DocContext, OperationDoc, Param, ParamsDoc, PartsDoc, path_params,
    stored_query_params_for,
};
use crate::extract::{Mapped, RepeatedQuery, Strict};

/// What one handler argument contributes to the operation.
///
/// `ExtractDoc::document` returns this value; the document builder owns the merge rules. A trait
/// never mutates the shared `OperationDoc` bag.
#[derive(Default)]
pub struct ArgDoc {
    pub params: Vec<Param>,
    pub body: Option<BodyDoc>,
    /// Stable key for the contract's body plans.
    pub body_type_name: Option<fn() -> &'static str>,
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
        self.body_type_name = None;
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
        if arg.body_type_name.is_some() {
            self.body_type_name = arg.body_type_name;
        }
        self.parts.extend(arg.parts);
        self.failures.extend(arg.failures);
        self.required_auth.extend(arg.required_auth);
        self.optional_auth.extend(arg.optional_auth);
        self.schemes.extend(arg.schemes);
        if arg.path_schema.is_some() {
            self.path_schema = arg.path_schema;
        }
        if arg.unverified_body {
            self.verification.mark_body();
        }
        if arg.unverified_extractor {
            self.verification.mark_extractor();
        }
        self.verification
            .extend_unbounded_file_fields(arg.unbounded_file_fields);
    }
}

/// One route argument's contribution to the document.
pub trait ExtractDoc: crate::arg::ExtractArg {
    /// The argument's documentation. Associated, because the macro describes the extractor type
    /// before a value exists.
    fn document(context: &mut DocContext<'_>) -> ArgDoc;
}

// --- Bodies -------------------------------------------------------------------------------

impl<T: ApiSchema + crate::doc::ValidateHook + 'static> ExtractDoc for axum::Json<T> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(json_body::<T>(context)),
            body_type_name: Some(|| std::any::type_name::<T>()),
            unverified_body: true,
            failures: json_failures(),
            ..ArgDoc::default()
        }
    }
}

impl<T: ApiSchema + crate::doc::ValidateHook + serde::Serialize + 'static> ExtractDoc
    for axum::Form<T>
{
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(form_body::<T>(context)),
            body_type_name: Some(|| std::any::type_name::<T>()),
            unverified_body: true,
            failures: form_failures(),
            ..ArgDoc::default()
        }
    }
}

impl ExtractDoc for bytes::Bytes {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(binary_body()),
            ..ArgDoc::default()
        }
    }
}

impl ExtractDoc for axum::body::Body {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            body: Some(binary_body()),
            ..ArgDoc::default()
        }
    }
}

// A plain `String` body is a raw request body the framework cannot describe; keep the current
// honesty marker rather than guessing at a media type.
impl ExtractDoc for String {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_extractor: true,
            ..ArgDoc::default()
        }
    }
}

// A `Request` hands the whole request to the handler. The framework cannot know how the body
// will be read, so the operation keeps the unverified-body marker instead of claiming no body.
impl ExtractDoc for axum::extract::Request {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_body: true,
            ..ArgDoc::default()
        }
    }
}

#[cfg(feature = "multipart")]
impl<F: crate::multipart::MultipartDoc + 'static> ExtractDoc for crate::multipart::Multipart<F> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        let mut doc = ArgDoc {
            body: Some(multipart_body::<F>(context)),
            body_type_name: Some(|| std::any::type_name::<F>()),
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
impl ExtractDoc for axum::extract::Multipart {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_body: true,
            ..ArgDoc::default()
        }
    }
}

impl<F, M> ExtractDoc for Result<F, M>
where
    F: ExtractDoc,
{
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        // A `Result<Extractor, E>` owns its rejection. The handler receives the success value or
        // its own error type, so the route shim (and the raw body layer) must not enforce the
        // schema a second time. Keep the documented shape, drop the runtime registration.
        F::document(context).result()
    }
}

// --- Parts --------------------------------------------------------------------------------

impl<T: ParamsDoc + crate::doc::ValidateHook + 'static> ExtractDoc for axum::extract::Path<T> {
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
}

impl<T: ParamsDoc + crate::doc::ValidateHook + 'static> ExtractDoc for axum::extract::Query<T> {
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
}

impl<T: ParamsDoc + crate::doc::ValidateHook + 'static> ExtractDoc for RepeatedQuery<T> {
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
}

impl<T> ExtractDoc for axum::extract::Extension<T> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_extractor: true,
            ..ArgDoc::default()
        }
    }
}

impl<S> ExtractDoc for axum::extract::State<S> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc::default()
    }
}

#[cfg(feature = "serve")]
impl<T> ExtractDoc for axum::extract::ConnectInfo<T> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc::default()
    }
}

macro_rules! no_op_extractor {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ExtractDoc for $ty {
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

impl<A, S> ExtractDoc for Auth<A, S>
where
    A: AuthScheme,
    S: Scopes<A>,
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

impl<T: ExtractDoc> ExtractDoc for Option<T> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        T::document(context).optional()
    }
}

impl<T> ExtractDoc for Mapped<T> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        // `Mapped` only maps a rejection; it deliberately does not document or enforce a schema.
        ArgDoc {
            unverified_extractor: true,
            ..ArgDoc::default()
        }
    }
}

impl<T: ApiSchema + crate::doc::ValidateHook + 'static> ExtractDoc for Strict<axum::Json<T>> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::Json<T> as ExtractDoc>::document(context)
    }
}

impl<T: ApiSchema + crate::doc::ValidateHook + serde::Serialize + 'static> ExtractDoc
    for Strict<axum::Form<T>>
{
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::Form<T> as ExtractDoc>::document(context)
    }
}

impl<T: ParamsDoc + crate::doc::ValidateHook + 'static> ExtractDoc
    for Strict<axum::extract::Path<T>>
{
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::extract::Path<T> as ExtractDoc>::document(context)
    }
}

impl<T: ParamsDoc + crate::doc::ValidateHook + 'static> ExtractDoc
    for Strict<axum::extract::Query<T>>
{
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <axum::extract::Query<T> as ExtractDoc>::document(context)
    }
}

impl<T> ExtractDoc for Strict<axum::extract::Extension<T>> {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_extractor: true,
            failures: extension_failures(),
            ..ArgDoc::default()
        }
    }
}

#[cfg(feature = "multipart")]
impl<F: crate::multipart::MultipartDoc + 'static> ExtractDoc
    for Strict<crate::multipart::Multipart<F>>
{
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        <crate::multipart::Multipart<F> as ExtractDoc>::document(context)
    }
}
