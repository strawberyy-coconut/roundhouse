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

#[roundhouse::async_trait]
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
}

/// Additional header auth used to demonstrate an AND requirement.
pub struct ApiKeyAuth;

impl AuthScheme for ApiKeyAuth {
    type State = AppState;
    type Identity = ();

    const NAME: &'static str = "api-key";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::ApiKeyHeader("X-API-Key");
}

#[roundhouse::async_trait]
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
}

/// OAuth scope marker: `Auth<BearerAuth, ReadPosts>`.
pub struct ReadPosts;
impl roundhouse::AuthScopes for ReadPosts {
    const SCOPES: &'static [&'static str] = &["read:posts"];
}

pub struct WritePosts;
impl roundhouse::AuthScopes for WritePosts {
    const SCOPES: &'static [&'static str] = &["write:posts"];
}
