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
//! (`validate = false`); the shim asks each argument's
//! [`ExtractArg`](crate::doc::ExtractArg) implementation whether it has work to do.
//!
//! The same seam owns the rejection contract: an argument whose extractor is one of axum's
//! fallible built-ins is read through [`Mapped`](crate::extract::Mapped), so the rejection is a
//! [`Problem`](crate::Problem) rather than axum's plain-text response. The handler still declares
//! and destructures its own type; only the shim's copy is wrapped.
//!
//! An explicit [`Strict`](crate::extract::Strict) is left alone: that extractor enforces the schema
//! itself, and only the `validate_with` hooks fall through to its `ExtractArg` implementation.

use axum::extract::FromRequestParts;

/// A state type that can be extracted from a handler's declared router state.
///
/// `#[handler]` emits this bound for every `State<T>` argument. `routes!` instantiates the shim
/// with the table's `state = ...` type (or the handler's inferred default), so a substate only
/// needs axum's ordinary `FromRef` implementation.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be extracted from the handler's router state `{S}`",
    label = "`{Self}` does not implement `axum::extract::FromRef<{S}>`",
    note = "implement `FromRef<{S}>` for `{Self}` (or derive `FromRef` on the app state), and set the `routes!` table's `state = {S}`"
)]
pub trait SubstateOf<S>: axum::extract::FromRef<S> {}

impl<S, T> SubstateOf<S> for T where T: axum::extract::FromRef<S> {}

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
