//! The runtime extractor contract used by the route shim.
//!
//! `#[handler]` asks each extractor whether it consumes the body and whether the shim should
//! re-validate it. Documentation is a separate, contract-only trait (`doc::ExtractDoc`), so a
//! production build with the `contract` feature off compiles no schema machinery.

use crate::auth::{Auth, AuthScheme, Scopes};
use crate::doc::ValidateHook;
use crate::extract::{Mapped, RepeatedQuery, Strict, ValidationContext};
use crate::problem::Problem;

#[cfg(feature = "multipart")]
use crate::multipart::MultipartForm;

/// One route argument's runtime contribution.
///
/// Implement this for an extractor type to make it first-class in a route. The default `validate`
/// is a no-op, which is right for extractors that enforce their own contract (or have no request
/// constraints).
pub trait ExtractArg {
    /// Whether this extractor consumes the request body. The handler macro uses this only to keep
    /// the validation marker honest; axum still enforces the one-body rule and the argument order.
    const IS_BODY: bool = false;

    /// Whether the route shim re-checks this body after extraction. Body extractors that already
    /// executed the schema during extraction (`Strict`, multipart) set this to `false`.
    const SHIM_VALIDATES: bool = false;

    /// Route-level validation after extraction. The default leaves the value alone.
    #[allow(clippy::result_large_err)]
    fn validate(&self, _context: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn validation_problem(
    errors: crate::validate::ValidationErrors,
    context: &ValidationContext,
) -> Problem {
    crate::extract::validation_problem(errors, context.status())
}

#[allow(clippy::result_large_err)]
pub(crate) fn validate_body<T>(value: &T, context: &ValidationContext) -> Result<(), Problem>
where
    T: ValidateHook + serde::Serialize + 'static,
{
    crate::extract::enforce_schema(value, context.engine(), context.status())?;
    ValidateHook::validate(value).map_err(|errors| validation_problem(errors, context))
}

#[allow(clippy::result_large_err)]
pub(crate) fn validate_parts<T>(value: &T, context: &ValidationContext) -> Result<(), Problem>
where
    T: crate::doc::Params + ValidateHook + 'static,
{
    crate::extract::enforce_parts_schema(value, context.engine(), context.status())?;
    ValidateHook::validate(value).map_err(|errors| validation_problem(errors, context))
}

// --- Bodies -------------------------------------------------------------------------------

impl<T: ValidateHook + 'static> ExtractArg for axum::Json<T> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        // The raw JSON layer already executed the declared schema; only the custom hooks remain.
        ValidateHook::validate(&self.0).map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: ValidateHook + serde::Serialize + 'static> ExtractArg for axum::Form<T> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_body(&self.0, context)
    }
}

impl ExtractArg for bytes::Bytes {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;
}

impl ExtractArg for axum::body::Body {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;
}

impl ExtractArg for String {
    const IS_BODY: bool = true;
}

impl ExtractArg for axum::extract::Request {
    const IS_BODY: bool = true;
}

#[cfg(feature = "multipart")]
impl<F: MultipartForm + 'static> ExtractArg for crate::multipart::Multipart<F> {
    const IS_BODY: bool = true;
}

#[cfg(feature = "multipart")]
impl ExtractArg for axum::extract::Multipart {
    const IS_BODY: bool = true;
}

impl<F, M> ExtractArg for Result<F, M>
where
    F: ExtractArg,
{
    const IS_BODY: bool = F::IS_BODY;
}

// --- Parts --------------------------------------------------------------------------------

impl<T: crate::doc::Params + ValidateHook + 'static> ExtractArg for axum::extract::Path<T> {
    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)
    }
}

impl<T: crate::doc::Params + ValidateHook + 'static> ExtractArg for axum::extract::Query<T> {
    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)
    }
}

impl<T: crate::doc::Params + ValidateHook + 'static> ExtractArg for RepeatedQuery<T> {}

impl<T> ExtractArg for axum::extract::Extension<T> {}

impl<S> ExtractArg for axum::extract::State<S> {}

#[cfg(feature = "serve")]
impl<T> ExtractArg for axum::extract::ConnectInfo<T> {}

macro_rules! no_op_extractor {
    ($($ty:ty),* $(,)?) => {
        $(impl ExtractArg for $ty {})*
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
    S: Scopes<A>,
{
}

// --- Wrappers -----------------------------------------------------------------------------

impl<T: ExtractArg> ExtractArg for Option<T> {
    const IS_BODY: bool = T::IS_BODY;
    const SHIM_VALIDATES: bool = T::SHIM_VALIDATES;

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        match self {
            Some(value) => value.validate(context),
            None => Ok(()),
        }
    }
}

impl<T> ExtractArg for Mapped<T> {}

impl<T: ValidateHook + 'static> ExtractArg for Strict<axum::Json<T>> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        // `Strict` already executed the schema; run only the custom hooks.
        ValidateHook::validate(&self.0.0).map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: ValidateHook + serde::Serialize + 'static> ExtractArg for Strict<axum::Form<T>> {
    const IS_BODY: bool = true;
    const SHIM_VALIDATES: bool = true;

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        ValidateHook::validate(&self.0.0).map_err(|errors| validation_problem(errors, context))
    }
}

impl<T: crate::doc::Params + ValidateHook + 'static> ExtractArg for Strict<axum::extract::Path<T>> {
    // `Strict` already enforced the parts schema during extraction.
}

impl<T: crate::doc::Params + ValidateHook + 'static> ExtractArg
    for Strict<axum::extract::Query<T>>
{
}

impl<T> ExtractArg for Strict<axum::extract::Extension<T>> {}

#[cfg(feature = "multipart")]
impl<F: MultipartForm + 'static> ExtractArg for Strict<crate::multipart::Multipart<F>> {}
