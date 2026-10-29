use axum::http::StatusCode;
use axum::http::header;
use axum::http::request::Parts;
use roundhouse::{AuthScheme, Authenticate, Problem, SecuritySchemeKind};

use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct DemoUser {
    pub name: String,
}

/// Pure scheme metadata. The authenticated principal is `DemoUser`.
pub struct DemoBearer;

impl AuthScheme for DemoBearer {
    type State = AppState;
    type Identity = DemoUser;
    const NAME: &'static str = "bearer";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::HttpBearer;
}

#[roundhouse::async_trait]
impl Authenticate for DemoBearer {
    async fn authenticate(parts: &mut Parts, state: &AppState) -> Result<DemoUser, Problem> {
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        token
            .and_then(|token| state.authorize(token))
            .ok_or_else(|| {
                Problem::new(StatusCode::UNAUTHORIZED, "Unauthorized")
                    .type_uri("https://roundhouse.rs/problems/Unauthorized")
                    .detail("expected a bearer token from the app state token store")
            })
    }
}
