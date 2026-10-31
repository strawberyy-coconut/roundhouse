//! The generated handler shim: route-level policy applied without touching the handler.
//!
//! `#[handler]` emits an associated `__roundhouse_shim::<P>` beside the marker type. The shim has
//! the handler's own extractors plus the policy's auth extractor and a [`ValidationContext`], so
//! the route table can choose a policy at the call site:
//!
//! ```ignore
//! axum::routing::get(hello_marker::__roundhouse_shim::<roundhouse::shim::Plain>)
//! ```
//!
//! The handler function itself stays exactly as written. Validation is opt-out per route
//! (`validate = false`); the shim checks the args that the framework's own extractors do not
//! already validate, which is why [`ValidateArg`] is a no-op for `roundhouse::Json` and friends.
//!
//! The same seam owns the rejection contract: an argument whose extractor is one of axum's
//! fallible built-ins is read through [`Mapped`](crate::extract::Mapped), so the rejection is a
//! [`Problem`](crate::Problem) rather than axum's plain-text response. The handler still declares
//! and destructures its own type; only the shim's copy is wrapped.
//!
//! An explicit [`Strict`](crate::extract::Strict) is left alone: that extractor enforces the schema
//! itself, and only the `validate_with` hooks fall through to [`ValidateArg`] here.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::extract::{SkipValidation, ValidationConfig};
use crate::problem::Problem;

impl ValidationContext {
    /// Whether validation should run for this request: the route did not opt out and the app
    /// did not disable the engine.
    pub fn enabled(&self) -> bool {
        !self.skip && self.config.mode != crate::extract::Validation::Off
    }
}

/// The app-wide validation settings, read from the request.
#[derive(Clone)]
pub struct ValidationContext {
    pub config: ValidationConfig,
    /// Set by a `validate = false` route or an ancestor that opted out.
    pub skip: bool,
    /// The compiled document, when the app enables schema execution.
    pub engine: Option<std::sync::Arc<roundhouse_schema::engine::SchemaEngine>>,
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
        let engine = parts
            .extensions
            .get::<std::sync::Arc<roundhouse_schema::engine::SchemaEngine>>()
            .cloned();
        Ok(Self {
            config,
            skip,
            engine,
        })
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
}

/// A policy whose state is exactly a stateful handler's `State<S>`.
///
/// A stateless handler can use any scheme state; a handler that declares `State<S>` pins it, and
/// a scheme whose `AuthScheme::State` differs is a compile error rather than a startup one.
#[diagnostic::on_unimplemented(
    message = "the route's security scheme does not authenticate against the handler's state",
    label = "the scheme's `AuthScheme::State` must equal the handler's `State<T>`",
    note = "use a scheme whose `State` is the handler's state, add `State<Scheme::State>` to the \
            handler, or declare `security` on the module that owns the route"
)]
pub trait ShimPolicyFor<S: Clone + Send + Sync + 'static>: ShimPolicy<State = S> {}

impl<S, P> ShimPolicyFor<S> for P
where
    S: Clone + Send + Sync + 'static,
    P: ShimPolicy<State = S>,
{
}

/// No security scheme: the default route.
pub struct Plain<S>(std::marker::PhantomData<S>);

/// A route entry with one security scheme.
pub struct Authed<A>(std::marker::PhantomData<A>);

impl<S> ShimPolicy for Plain<S>
where
    S: Clone + Send + Sync + 'static,
{
    type State = S;
    type Auth = ();
}

impl<A: crate::auth::Authenticate> ShimPolicy for Authed<A> {
    type State = A::State;
    type Auth = crate::auth::AuthEnforce<A>;
}

/// One argument's contribution to route-level validation.
///
/// The shim calls this for every argument. The fallback `impl for &T` means an extractor the
/// framework does not know is left alone rather than failing to compile; the framework's own
/// extractors are deliberately no-ops because they validate during extraction.
#[allow(clippy::result_large_err)]
pub trait ValidateArg {
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem>;
}

fn validation_problem(
    errors: crate::validate::ValidationErrors,
    context: &ValidationContext,
) -> Problem {
    crate::extract::validation_problem(errors, context.config.status)
}

#[allow(clippy::result_large_err)]
fn validate_body<T>(value: &T, context: &ValidationContext) -> Result<(), Problem>
where
    T: crate::doc::ApiSchema + serde::Serialize + 'static,
{
    crate::extract::enforce_schema(
        value,
        context.engine.as_ref(),
        context.config.status,
    )?;
    value
        .validate()
        .map_err(|errors| validation_problem(errors, context))
}

#[allow(clippy::result_large_err)]
fn validate_parts<T>(value: &T, context: &ValidationContext) -> Result<(), Problem>
where
    T: crate::doc::PartsValue + 'static,
{
    crate::extract::enforce_parts_schema(
        value,
        context.engine.as_ref(),
        context.config.status,
    )
}

// `axum::Json` parses but does not execute the schema; the route's shim does.
impl<T> ValidateArg for axum::Json<T>
where
    T: crate::doc::ApiSchema + 'static,
{
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem> {
        // The raw JSON layer already executed the schema; only the custom hooks remain.
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T> ValidateArg for axum::Form<T>
where
    T: crate::doc::ApiSchema + serde::Serialize + 'static,
{
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_body(&self.0, context)
    }
}

impl<T> ValidateArg for axum::extract::Path<T>
where
    T: crate::doc::PathValidate + crate::doc::PartsValue + 'static,
{
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)?;
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T> ValidateArg for axum::extract::Query<T>
where
    T: crate::doc::QueryParams + crate::doc::PartsValue + 'static,
{
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem> {
        validate_parts(&self.0, context)?;
        self.0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

// An explicit `Strict` extractor already ran the schema in the extractor; only the custom hooks
// remain, and they need the concrete type. `Path`/`Query`/`Extension`/`Multipart` are complete, so
// the fallback below leaves them alone.
impl<T> ValidateArg for crate::extract::Strict<axum::Json<T>>
where
    T: crate::doc::ApiSchema,
{
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem> {
        self.0
            .0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

impl<T> ValidateArg for crate::extract::Strict<axum::Form<T>>
where
    T: crate::doc::ApiSchema,
{
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem> {
        self.0
            .0
            .validate()
            .map_err(|errors| validation_problem(errors, context))
    }
}

// `roundhouse::Query` still parses and validates during extraction; the shim leaves it alone.
impl<T> ValidateArg for crate::extract::RepeatedQuery<T> {
    fn validate_arg(&self, _context: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }
}

impl<T> ValidateArg for Option<T>
where
    T: ValidateArg,
{
    fn validate_arg(&self, context: &ValidationContext) -> Result<(), Problem> {
        match self {
            Some(value) => value.validate_arg(context),
            Option::None => Ok(()),
        }
    }
}

/// The fallback: an extractor with no documented constraints, or one the framework cannot see.
impl<T> ValidateArg for &T {
    fn validate_arg(&self, _context: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }
}

/// Type-level probe used by `#[handler]` to decide whether a body the shim validated is still
/// "unverified".
///
/// The probe must be invoked at the handler's concrete argument type (with [`ProbeBody`] in
/// scope), not from inside a generic helper: method resolution happens before monomorphization,
/// so a generic wrapper would always pick the fallback.
pub struct BodyProbe<T>(std::marker::PhantomData<T>);

impl<T> BodyProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for BodyProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeBody {
    fn probe_body(self) -> bool;
}

impl<T: utoipa::ToSchema> ProbeBody for &BodyProbe<axum::Json<T>> {
    fn probe_body(self) -> bool {
        true
    }
}

impl<T: crate::doc::ApiSchema + serde::Serialize> ProbeBody for &BodyProbe<axum::Form<T>> {
    fn probe_body(self) -> bool {
        true
    }
}

impl<T: utoipa::ToSchema + serde::Serialize> ProbeBody
    for &BodyProbe<crate::extract::Strict<axum::Json<T>>>
{
    fn probe_body(self) -> bool {
        true
    }
}

impl<T: crate::doc::ApiSchema + serde::Serialize> ProbeBody
    for &BodyProbe<crate::extract::Strict<axum::Form<T>>>
{
    fn probe_body(self) -> bool {
        true
    }
}

// A raw body has no constraints to enforce; it is already an honest shape.
impl ProbeBody for &BodyProbe<bytes::Bytes> {
    fn probe_body(self) -> bool {
        true
    }
}

impl ProbeBody for &BodyProbe<axum::body::Body> {
    fn probe_body(self) -> bool {
        true
    }
}

// Unknown extractors, `Result<Extractor, E>` bodies and multipart stay unverified.
impl<T> ProbeBody for &&BodyProbe<T> {
    fn probe_body(self) -> bool {
        false
    }
}
