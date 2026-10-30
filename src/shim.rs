//! The generated handler shim: route-level policy applied without touching the handler.
//!
//! `#[handler]` emits an associated `__roundhouse_shim::<P>` beside the marker type. The shim has
//! the handler's own extractors plus the policy's auth extractor and a [`ValidationContext`], so
//! the route table can choose a policy at the call site:
//!
//! ```ignore
//! axum::routing::get(hello_marker::__roundhouse_shim::<roundhouse::shim::Validated>)
//! ```
//!
//! The handler function itself stays exactly as written. Validation is opt-out per route
//! (`validate = false`); the shim checks the args that the framework's own extractors do not
//! already validate, which is why [`ValidateArg`] is a no-op for `roundhouse::Json` and friends.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::extract::{SkipValidation, ValidationConfig};
use crate::problem::Problem;

/// The app-wide validation settings, read from the request.
#[derive(Debug, Clone, Copy)]
pub struct ValidationContext {
    pub config: ValidationConfig,
    /// Set by a `validate = false` route or an ancestor that opted out.
    pub skip: bool,
}

impl<S> FromRequestParts<S> for ValidationContext
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let config = parts
            .extensions
            .get::<ValidationConfig>()
            .copied()
            .unwrap_or_default();
        let skip = parts.extensions.get::<SkipValidation>().is_some();
        Ok(Self { config, skip })
    }
}

/// The policy a generated shim is instantiated with.
///
/// `Auth` is the extractor the shim adds for the route's security requirement; `()` means "the
/// route declares no scheme", so the common case costs one zero-sized extractor.
pub trait ShimPolicy: 'static {
    /// The state the shim extracts. For an authenticated route this is the scheme's state, so a
    /// stateless handler can sit under `security = [A]` without declaring `State<A::State>`.
    type State: Clone + Send + Sync + 'static;
    type Auth: FromRequestParts<Self::State> + Send + 'static;

    /// Whether the shim validates every validatable argument.
    const VALIDATE: bool;
}

/// No validation, no auth: the `validate = false` route with no security requirement.
pub struct Plain<S>(std::marker::PhantomData<S>);

/// Validation on, no auth: the default.
pub struct Validated<S>(std::marker::PhantomData<S>);

/// A route entry with one security scheme and validation off.
pub struct Authed<A>(std::marker::PhantomData<A>);

/// A route entry with one security scheme and validation on.
pub struct AuthedValidated<A>(std::marker::PhantomData<A>);

impl<S> ShimPolicy for Plain<S>
where
    S: Clone + Send + Sync + 'static,
{
    type State = S;
    type Auth = ();
    const VALIDATE: bool = false;
}

impl<S> ShimPolicy for Validated<S>
where
    S: Clone + Send + Sync + 'static,
{
    type State = S;
    type Auth = ();
    const VALIDATE: bool = true;
}

impl<A: crate::auth::Authenticate> ShimPolicy for Authed<A> {
    type State = A::State;
    type Auth = crate::auth::AuthEnforce<A>;
    const VALIDATE: bool = false;
}

impl<A: crate::auth::Authenticate> ShimPolicy for AuthedValidated<A> {
    type State = A::State;
    type Auth = crate::auth::AuthEnforce<A>;
    const VALIDATE: bool = true;
}

/// One argument's contribution to route-level validation.
///
/// The shim calls this for every argument. The fallback `impl for &T` means an extractor the
/// framework does not know is left alone rather than failing to compile; the framework's own
/// extractors are deliberately no-ops because they validate during extraction.
#[allow(clippy::result_large_err)]
pub trait ValidateArg {
    fn validate_arg(&self, config: &ValidationConfig) -> Result<(), Problem>;
}

fn validation_problem(
    errors: crate::validate::ValidationErrors,
    config: &ValidationConfig,
) -> Problem {
    crate::extract::validation_problem(errors, config.status)
}

// `axum::Json` parses but does not enforce the constraints in `T`'s schema.
impl<T> ValidateArg for axum::Json<T>
where
    T: crate::doc::ApiSchema,
{
    fn validate_arg(&self, config: &ValidationConfig) -> Result<(), Problem> {
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, config))
    }
}

impl<T> ValidateArg for axum::Form<T>
where
    T: crate::doc::ApiSchema,
{
    fn validate_arg(&self, config: &ValidationConfig) -> Result<(), Problem> {
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, config))
    }
}

impl<T> ValidateArg for axum::extract::Path<T>
where
    T: crate::doc::PathValidate,
{
    fn validate_arg(&self, config: &ValidationConfig) -> Result<(), Problem> {
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, config))
    }
}

impl<T> ValidateArg for axum::extract::Query<T>
where
    T: crate::doc::QueryParams,
{
    fn validate_arg(&self, config: &ValidationConfig) -> Result<(), Problem> {
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, config))
    }
}

// The framework's own extractors already validate during extraction; validating again would
// produce a second, identical pass.
impl<T> ValidateArg for crate::extract::Json<T> {
    fn validate_arg(&self, _config: &ValidationConfig) -> Result<(), Problem> {
        Ok(())
    }
}

impl<T> ValidateArg for crate::extract::Form<T> {
    fn validate_arg(&self, _config: &ValidationConfig) -> Result<(), Problem> {
        Ok(())
    }
}

impl<T> ValidateArg for crate::extract::Path<T> {
    fn validate_arg(&self, _config: &ValidationConfig) -> Result<(), Problem> {
        Ok(())
    }
}

impl<T> ValidateArg for crate::extract::Query<T> {
    fn validate_arg(&self, _config: &ValidationConfig) -> Result<(), Problem> {
        Ok(())
    }
}

impl<T> ValidateArg for Option<T>
where
    T: ValidateArg,
{
    fn validate_arg(&self, config: &ValidationConfig) -> Result<(), Problem> {
        match self {
            Some(value) => value.validate_arg(config),
            Option::None => Ok(()),
        }
    }
}

/// The fallback: an extractor with no documented constraints, or one the framework cannot see.
impl<T> ValidateArg for &T {
    fn validate_arg(&self, _config: &ValidationConfig) -> Result<(), Problem> {
        Ok(())
    }
}
