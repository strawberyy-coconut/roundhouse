//! `Auth<A>` extractors and the security-scheme registry.

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::StatusCode;
use axum::http::header;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use std::future::Future;
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

pub fn auth_doc<A: AuthScheme, Scopes: AuthScopes>() -> AuthDoc {
    AuthDoc {
        name: A::NAME,
        scopes: Scopes::SCOPES,
    }
}

/// Pure scheme metadata for `Auth<A>`.
///
/// This type never carries the authenticated user. It only names the scheme, describes it in the
/// document, and declares the rejection statuses.
pub trait AuthScheme: Sized + Send + Sync + 'static {
    /// The app state this scheme authenticates against. A handler using `Auth<A>` pins its
    /// `MethodRouter` to this type, just like `State<Self::State>`.
    type State: Send + Sync + 'static;

    /// The authenticated principal returned by `Auth<A>`.
    type Identity: Send + Sync + 'static;

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

/// OAuth scopes carried by an `Auth<A, Scopes>` signature.
pub trait AuthScopes {
    const SCOPES: &'static [&'static str] = &[];
}

impl AuthScopes for NoScopes {}

/// The authorization decision behind a scoped `Auth<A, Scopes>`.
///
/// A scope marker is published to the document in a security requirement. Implementing this
/// trait is the handler's proof that the requirement is enforced: `Auth<A, Scopes>` does not
/// compile without it. `NoScopes` is the only built-in implementation and grants everything.
pub trait Authorize<A: AuthScheme>: AuthScopes {
    /// Return `Ok(())` when `identity` may exercise this route, or the rejection to send.
    #[allow(clippy::result_large_err)]
    fn authorize(identity: &A::Identity) -> Result<(), Problem>;
}

impl<A: AuthScheme> Authorize<A> for NoScopes {
    #[allow(clippy::result_large_err)]
    fn authorize(_identity: &A::Identity) -> Result<(), Problem> {
        Ok(())
    }
}

/// The authenticated principal, tagged with the scheme that produced it.
pub struct Auth<A: AuthScheme, Scopes = NoScopes> {
    pub principal: A::Identity,
    pub scopes: std::marker::PhantomData<Scopes>,
}

impl<A: AuthScheme, Scopes> Auth<A, Scopes> {
    pub fn into_inner(self) -> A::Identity {
        self.principal
    }
}

impl<A: AuthScheme, Scopes> std::ops::Deref for Auth<A, Scopes> {
    type Target = A::Identity;
    fn deref(&self) -> &Self::Target {
        &self.principal
    }
}

#[allow(clippy::manual_async_fn)]
impl<A, Scopes> FromRequestParts<A::State> for Auth<A, Scopes>
where
    A: Authenticate,
    Scopes: Authorize<A> + Send + Sync + 'static,
{
    type Rejection = AuthRejection;

    fn from_request_parts(
        parts: &mut Parts,
        state: &A::State,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async move {
            match A::authenticate(parts, state).await {
                Ok(principal) => {
                    Scopes::authorize(&principal).map_err(|problem| AuthRejection {
                        problem,
                        scheme: A::KIND,
                    })?;
                    Ok(Auth {
                        principal,
                        scopes: std::marker::PhantomData,
                    })
                }
                Err(problem) => Err(AuthRejection {
                    problem,
                    scheme: A::KIND,
                }),
            }
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl<A, Scopes> OptionalFromRequestParts<A::State> for Auth<A, Scopes>
where
    A: Authenticate,
    Scopes: Authorize<A> + Send + Sync + 'static,
{
    type Rejection = AuthRejection;

    fn from_request_parts(
        parts: &mut Parts,
        state: &A::State,
    ) -> impl std::future::Future<Output = Result<Option<Self>, Self::Rejection>> + Send {
        async move {
            if !A::presents_credentials(parts) {
                return Ok(None);
            }
            <Auth<A, Scopes> as FromRequestParts<A::State>>::from_request_parts(parts, state)
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

/// The required security requirement for an `Auth<A, Scopes>` handler argument.
pub fn auth_requirement<A: AuthScheme, Scopes: AuthScopes>() -> SecurityRequirement {
    SecurityRequirement::new(A::NAME, Scopes::SCOPES.iter().copied())
}

/// Optional security is OpenAPI's two alternatives: the scheme, or no scheme.
pub fn optional_auth_requirements<A: AuthScheme, Scopes: AuthScopes>() -> Vec<SecurityRequirement> {
    vec![
        auth_requirement::<A, Scopes>(),
        SecurityRequirement::default(),
    ]
}

pub fn auth_scheme<A: AuthScheme>() -> SchemeDoc {
    SchemeDoc {
        name: A::NAME,
        kind: A::KIND,
    }
}
