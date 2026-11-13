//! Authentication and authorization: `Auth<A>` extractors and the security-scheme registry.
//!
//! An [`AuthScheme`] declares its name, kind, state and identity type. Its [`Authenticate`]
//! implementation reads credentials — with access to `Self::State`, normally the app state — and
//! returns an identity or a rejection.
//!
//! `Auth<A>` extracts that identity. `Auth<A, Scopes>` additionally publishes `Scopes` in the
//! operation's security requirement and only compiles when `S: Scopes<A>`. Several
//! required `Auth<A>` extractors AND into one requirement; `Option<Auth<A>>` adds the
//! unauthenticated alternative.
//!
//! Module-level `security = [A, ...]` names scheme types. It installs the `Auth<A>` check on every
//! route below that module and publishes the matching requirement.
//!
//! A `routes!` entry with one scheme is enforced by the generated handler shim instead: the
//! shim extracts [`AuthEnforce<A>`] before the body, and the route's guard is skipped
//! (`RouteRecord::shim_auth`). If the handler also takes `Auth<A>` (or a module guard already
//! authenticated), the successful extraction leaves an [`Authenticated<A>`] marker in the
//! request extensions and the shim's enforcement is a no-op, so one request authenticates at
//! most once per scheme.
//!
//! The schemes in `examples/auth` are a compact reference implementation. Production code should
//! verify tokens with a maintained library or gateway.

use std::any::{Any, TypeId};
use std::future::Future;

use axum::extract::{FromRequestParts, OptionalFromRequestParts, Request, State};
use axum::http::StatusCode;
use axum::http::header;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;
use utoipa::openapi::SecurityRequirement;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};

use crate::problem::Problem;

/// The scheme metadata an `AuthScheme` contributes to `components.securitySchemes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecuritySchemeKind {
    HttpBearer,
    HttpBasic,
    ApiKeyHeader(&'static str),
    ApiKeyQuery(&'static str),
}

impl SecuritySchemeKind {
    pub fn to_utoipa(self) -> SecurityScheme {
        match self {
            SecuritySchemeKind::HttpBearer => {
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer))
            }
            SecuritySchemeKind::HttpBasic => SecurityScheme::Http(Http::new(HttpAuthScheme::Basic)),
            SecuritySchemeKind::ApiKeyHeader(name) => {
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new(name)))
            }
            SecuritySchemeKind::ApiKeyQuery(name) => {
                SecurityScheme::ApiKey(ApiKey::Query(ApiKeyValue::new(name)))
            }
        }
    }
}

/// One scheme declaration, keyed by name in `components.securitySchemes`.
#[derive(Debug, Clone, Copy)]
pub struct SchemeDoc {
    pub name: &'static str,
    pub kind: SecuritySchemeKind,
}

/// An auth requirement contributed by a handler or layer.
#[derive(Debug, Clone, Copy)]
pub struct AuthDoc {
    pub name: &'static str,
    pub scopes: &'static [&'static str],
}

pub fn auth_doc<A: AuthScheme, S: Scopes<A>>() -> AuthDoc {
    AuthDoc {
        name: A::NAME,
        scopes: S::NAMES,
    }
}

/// A security requirement that also installs its own enforcer.
///
/// A module's `security = [Bearer]` and a `routes!` entry's `security = [Bearer]` expand to these.
/// The guard is applied to the route's `MethodRouter`, so the declaration and the check share one
/// source.
#[derive(Clone, Copy)]
pub struct AuthGuard {
    pub name: &'static str,
    pub scheme: SchemeDoc,
    pub(crate) state: fn() -> TypeId,
    pub(crate) state_name: fn() -> &'static str,
    pub(crate) failures: fn() -> Vec<crate::doc::ResponseDoc>,
    pub(crate) apply: fn(MethodRouter<()>, &dyn Any) -> MethodRouter<()>,
}

impl AuthGuard {
    /// Construct a guard. Called by `routes!`; not part of the stable API.
    #[doc(hidden)]
    pub const fn __new<A: Authenticate>() -> Self {
        Self {
            name: A::NAME,
            scheme: SchemeDoc {
                name: A::NAME,
                kind: A::KIND,
            },
            state: state_id::<A>,
            state_name: state_name::<A>,
            failures: failures::<A>,
            apply: apply::<A>,
        }
    }

    pub(crate) fn doc(self) -> AuthDoc {
        AuthDoc {
            name: self.name,
            scopes: &[],
        }
    }
}

fn state_id<A: AuthScheme>() -> TypeId {
    TypeId::of::<A::State>()
}

fn state_name<A: AuthScheme>() -> &'static str {
    std::any::type_name::<A::State>()
}

fn failures<A: AuthScheme>() -> Vec<crate::doc::ResponseDoc> {
    crate::doc::auth_failures::<A>()
}

fn apply<A: Authenticate>(method_router: MethodRouter<()>, state: &dyn Any) -> MethodRouter<()> {
    let state = state
        .downcast_ref::<A::State>()
        .expect("roundhouse: auth guard state was validated at build time")
        .clone();
    method_router.layer(axum::middleware::from_fn_with_state(state, enforce::<A>))
}

async fn enforce<A: Authenticate>(
    State(state): State<A::State>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    match A::authenticate(&mut parts, &state).await {
        Ok(identity) => {
            // A generated shim or handler extractor that runs later reuses this identity.
            parts.extensions.insert(Authenticated::<A>(identity));
            next.run(Request::from_parts(parts, body)).await
        }
        Err(problem) => AuthRejection {
            problem,
            scheme: A::KIND,
        }
        .into_response(),
    }
}

/// A request-scoped marker: this scheme has already authenticated, plus the identity it produced.
///
/// It is inserted by `Auth<A>`, `Option<Auth<A>>` after a successful extraction and by a
/// module-level guard. A later extractor for the same scheme reuses the identity instead of
/// authenticating a second time, and a generated shim's [`AuthEnforce`] checks the marker.
pub struct Authenticated<A: AuthScheme>(A::Identity);

impl<A: AuthScheme> Clone for Authenticated<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Enforce a scheme for a route entry without taking its identity.
///
/// This is the extractor a `security = [A]` route entry installs when the handler did not ask for
/// `Auth<A>`. It is idempotent: if the handler (or a module guard) already authenticated, this is
/// a no-op.
pub struct AuthEnforce<A: Authenticate>(std::marker::PhantomData<A>);

#[allow(clippy::manual_async_fn)]
impl<A: Authenticate> FromRequestParts<A::State> for AuthEnforce<A> {
    type Rejection = AuthRejection;

    fn from_request_parts(
        parts: &mut Parts,
        state: &A::State,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            if parts.extensions.get::<Authenticated<A>>().is_some() {
                return Ok(AuthEnforce(std::marker::PhantomData));
            }
            let identity = A::authenticate(parts, state)
                .await
                .map_err(|problem| AuthRejection {
                    problem,
                    scheme: A::KIND,
                })?;
            parts.extensions.insert(Authenticated::<A>(identity));
            Ok(AuthEnforce(std::marker::PhantomData))
        }
    }
}

/// Pure scheme metadata for `Auth<A>`.
///
/// This type never carries the authenticated user. It only names the scheme, describes it in the
/// document, and declares the rejection statuses.
pub trait AuthScheme: Sized + Send + Sync + 'static {
    /// The app state this scheme authenticates against. A handler using `Auth<A>` pins its
    /// `MethodRouter` to this type, just like `State<Self::State>`.
    ///
    /// `Clone` is required so a module `security = [A]` guard can capture the state it
    /// authenticates against. Application states are already `Clone` for `App`.
    type State: Clone + Send + Sync + 'static;

    /// The authenticated principal returned by `Auth<A>`.
    ///
    /// `Clone` is required so route guards can cache a successful identity for a later
    /// extractor on the same request.
    type Identity: Clone + Send + Sync + 'static;

    const NAME: &'static str;
    const KIND: SecuritySchemeKind;
    const UNAUTHENTICATED: StatusCode = StatusCode::UNAUTHORIZED;
    const FORBIDDEN: Option<StatusCode> = Some(StatusCode::FORBIDDEN);
}

/// Authenticate against a specific application state.
///
/// This is separate from `AuthScheme` because the scheme is pure metadata while this trait needs
/// `&S` — usually `&AppState` — for a real token lookup. A handler using `Auth<A>` only compiles
/// when `A: Authenticate<AppState>`, keeping the same compile-time state check as `State<T>`.
pub trait Authenticate: AuthScheme {
    fn authenticate(
        parts: &mut Parts,
        state: &Self::State,
    ) -> impl Future<Output = Result<Self::Identity, Problem>> + Send;

    /// Whether the request carries credentials for this scheme at all.
    ///
    /// `Option<Auth<A>>` uses this to tell "anonymous" from "present but invalid": absent
    /// credentials yield `None`, bad ones yield the scheme's rejection.
    fn presents_credentials(parts: &Parts) -> bool;
}

/// Marker for an auth scheme without OAuth scopes.
pub struct NoScopes;

/// The scope names and authorization decision behind a scoped `Auth<A, Scopes>`.
///
/// The scope names are published to the document in a security requirement. Implementing this
/// trait is the handler's proof that the requirement is enforced: `Auth<A, Scopes>` does not
/// compile without it. `NoScopes` is the only built-in implementation and grants everything.
///
/// This trait is the route-level gate over the identity: the method receives only
/// `A::Identity`, so it can check roles, scopes or whether the principal is active. A policy that
/// needs path parameters or application state belongs in the handler after extraction; return a
/// `403 Problem` from there. The operation still publishes the route's scope, because the scope
/// gate and resource policy are separate checks.
pub trait Scopes<A: AuthScheme>: Send + Sync + 'static {
    /// The scope names published in the operation's security requirement.
    const NAMES: &'static [&'static str] = &[];

    /// Return `Ok(())` when `identity` may exercise this route, or the rejection to send.
    ///
    /// This method has no default on purpose: publishing a scope must be accompanied by an
    /// explicit authorization decision. Use [`NoScopes`] when authentication is the only check.
    #[allow(clippy::result_large_err)]
    fn authorize(identity: &A::Identity) -> Result<(), Problem>;
}

impl<A: AuthScheme> Scopes<A> for NoScopes {
    fn authorize(_identity: &A::Identity) -> Result<(), Problem> {
        Ok(())
    }
}

/// The authenticated principal, tagged with the scheme that produced it.
pub struct Auth<A: AuthScheme, S = NoScopes> {
    pub principal: A::Identity,
    pub scopes: std::marker::PhantomData<S>,
}

impl<A: AuthScheme, S> Auth<A, S> {
    pub fn into_inner(self) -> A::Identity {
        self.principal
    }
}

impl<A: AuthScheme, S> std::ops::Deref for Auth<A, S> {
    type Target = A::Identity;
    fn deref(&self) -> &Self::Target {
        &self.principal
    }
}

#[allow(clippy::manual_async_fn)]
impl<A, S> FromRequestParts<A::State> for Auth<A, S>
where
    A: Authenticate,
    S: Scopes<A>,
{
    type Rejection = AuthRejection;

    fn from_request_parts(
        parts: &mut Parts,
        state: &A::State,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            // A module guard may have authenticated already. Reuse its identity, but still run
            // this extractor's scope decision.
            let principal = match parts.extensions.get::<Authenticated<A>>() {
                Some(cached) => cached.0.clone(),
                None => A::authenticate(parts, state)
                    .await
                    .map_err(|problem| AuthRejection {
                        problem,
                        scheme: A::KIND,
                    })?,
            };
            S::authorize(&principal).map_err(|problem| AuthRejection {
                problem,
                scheme: A::KIND,
            })?;
            parts
                .extensions
                .insert(Authenticated::<A>(principal.clone()));
            Ok(Auth {
                principal,
                scopes: std::marker::PhantomData,
            })
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl<A, S> OptionalFromRequestParts<A::State> for Auth<A, S>
where
    A: Authenticate,
    S: Scopes<A>,
{
    type Rejection = AuthRejection;

    fn from_request_parts(
        parts: &mut Parts,
        state: &A::State,
    ) -> impl std::future::Future<Output = Result<Option<Self>, Self::Rejection>> + Send {
        async move {
            // A guard may have authenticated through a mechanism `presents_credentials` cannot
            // inspect (mTLS, a proxy header, ...). Once a guard has cached an identity, the
            // optional extractor observes it rather than reporting the request as anonymous.
            if let Some(cached) = parts.extensions.get::<Authenticated<A>>() {
                let principal = cached.0.clone();
                S::authorize(&principal).map_err(|problem| AuthRejection {
                    problem,
                    scheme: A::KIND,
                })?;
                return Ok(Some(Auth {
                    principal,
                    scopes: std::marker::PhantomData,
                }));
            }
            if !A::presents_credentials(parts) {
                return Ok(None);
            }
            <Auth<A, S> as FromRequestParts<A::State>>::from_request_parts(parts, state)
                .await
                .map(Some)
        }
    }
}

/// A rejection that keeps the `WWW-Authenticate` header for HTTP auth schemes.
pub struct AuthRejection {
    problem: Problem,
    scheme: SecuritySchemeKind,
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let mut response = self.problem.into_response();
        if response.status() == StatusCode::UNAUTHORIZED {
            let value = match self.scheme {
                SecuritySchemeKind::HttpBearer => Some("Bearer"),
                SecuritySchemeKind::HttpBasic => Some("Basic"),
                _ => None,
            };
            if let Some(value) = value {
                response.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    axum::http::HeaderValue::from_static(value),
                );
            }
        }
        response
    }
}

/// The required security requirement for an `Auth<A, S>` handler argument.
pub fn auth_requirement<A: AuthScheme, S: Scopes<A>>() -> SecurityRequirement {
    SecurityRequirement::new(A::NAME, S::NAMES.iter().copied())
}

/// Optional security is OpenAPI's two alternatives: the scheme, or no scheme.
pub fn optional_auth_requirements<A: AuthScheme, S: Scopes<A>>() -> Vec<SecurityRequirement> {
    vec![auth_requirement::<A, S>(), SecurityRequirement::default()]
}

pub fn auth_scheme<A: AuthScheme>() -> SchemeDoc {
    SchemeDoc {
        name: A::NAME,
        kind: A::KIND,
    }
}
