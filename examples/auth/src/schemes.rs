//! Reference implementation, not a supported backend.
//!
//! This module demonstrates the full `Auth<A>` surface: `Authenticate` and
//! `presents_credentials`, `Authorize` scope decisions, the 401/403 challenges produced by an
//! `AuthRejection`, and a two-scheme AND. It deliberately uses a tiny in-memory token store; use a
//! maintained identity library or gateway for production verification.

use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use roundhouse::{AuthScheme, Authenticate, Problem, SecuritySchemeKind};

use crate::AppState;

#[derive(Clone, Debug)]
pub struct User {
    pub name: String,
    pub admin: bool,
}

/// `Authorization: Bearer reader` or `Authorization: Bearer admin`.
pub struct BearerAuth;

impl AuthScheme for BearerAuth {
    type State = AppState;
    type Identity = User;

    const NAME: &'static str = "bearer";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::HttpBearer;
}

impl Authenticate for BearerAuth {
    async fn authenticate(parts: &mut Parts, state: &AppState) -> Result<User, Problem> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        token.and_then(|token| state.user(token)).ok_or_else(|| {
            Problem::new(StatusCode::UNAUTHORIZED, "Unauthorized")
                .type_uri("https://roundhouse.rs/problems/Unauthorized")
                .detail("expected a bearer token from the app state token store")
        })
    }

    fn presents_credentials(parts: &Parts) -> bool {
        parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("Bearer "))
    }
}

/// Additional header auth used to demonstrate an AND requirement.
pub struct ApiKeyAuth;

impl AuthScheme for ApiKeyAuth {
    type State = AppState;
    type Identity = ();

    const NAME: &'static str = "api-key";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::ApiKeyHeader("X-API-Key");
}

impl Authenticate for ApiKeyAuth {
    async fn authenticate(parts: &mut Parts, state: &AppState) -> Result<(), Problem> {
        let key = parts
            .headers
            .get("x-api-key")
            .and_then(|value| value.to_str().ok());
        if key.is_some_and(|key| state.api_key(key)) {
            Ok(())
        } else {
            Err(Problem::new(StatusCode::UNAUTHORIZED, "Unauthorized")
                .type_uri("https://roundhouse.rs/problems/Unauthorized")
                .detail("expected a valid `X-API-Key`"))
        }
    }

    fn presents_credentials(parts: &Parts) -> bool {
        parts.headers.contains_key("x-api-key")
    }
}

/// OAuth scope marker: `Auth<BearerAuth, ReadPosts>`.
pub struct ReadPosts;
impl roundhouse::AuthScopes for ReadPosts {
    const SCOPES: &'static [&'static str] = &["read:posts"];
}

impl roundhouse::Authorize<BearerAuth> for ReadPosts {
    fn authorize(_user: &User) -> Result<(), Problem> {
        // Every authenticated user of this example may read posts.
        Ok(())
    }
}

pub struct WritePosts;
impl roundhouse::AuthScopes for WritePosts {
    const SCOPES: &'static [&'static str] = &["write:posts"];
}

impl roundhouse::Authorize<BearerAuth> for WritePosts {
    fn authorize(user: &User) -> Result<(), Problem> {
        if user.admin {
            Ok(())
        } else {
            Err(Problem::new(StatusCode::FORBIDDEN, "Forbidden")
                .type_uri("https://roundhouse.rs/problems/Forbidden")
                .detail("write:posts requires the admin role"))
        }
    }
}
