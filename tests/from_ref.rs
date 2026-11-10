//! A table's `state = ...` declares the router state; a handler's `State<T>` then uses
//! axum's ordinary `FromRef` contract.

use axum::body::Body;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{Request, StatusCode, header};
use roundhouse::doc::{ExtractArg, HandlerDocs};
use roundhouse::prelude::*;
use roundhouse::utoipa::openapi::schema::Components;
use roundhouse::{App, BuildError};

#[derive(Clone)]
pub struct AppState {
    db: Db,
}

impl Default for AppState {
    fn default() -> Self {
        Self { db: Db("db") }
    }
}

#[derive(Clone, Default)]
pub struct Db(&'static str);

impl FromRef<AppState> for Db {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

#[derive(Clone, Default)]
pub struct OtherState;

pub struct Bearer;

impl AuthScheme for Bearer {
    type State = AppState;
    type Identity = ();

    const NAME: &'static str = "bearer";
    const KIND: roundhouse::SecuritySchemeKind = roundhouse::SecuritySchemeKind::HttpBearer;
}

impl Authenticate for Bearer {
    async fn authenticate(parts: &mut Parts, _state: &AppState) -> Result<(), Problem> {
        if parts
            .headers
            .get(header::AUTHORIZATION)
            .is_some_and(|value| value == "Bearer ok")
        {
            Ok(())
        } else {
            Err(Problem::new(StatusCode::UNAUTHORIZED, "Unauthorized"))
        }
    }

    fn presents_credentials(parts: &Parts) -> bool {
        parts.headers.contains_key(header::AUTHORIZATION)
    }
}

pub struct DbExtractor(Db);

impl FromRequestParts<AppState> for DbExtractor {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(Db::from_ref(state)))
    }
}

impl ExtractArg for DbExtractor {
    fn document(_components: &mut Components, _docs: &mut HandlerDocs) {}
}

fn app(module: &'static roundhouse::Module) -> roundhouse::Built {
    App::new(AppState::default())
        .include(module)
        .openapi(|| roundhouse::openapi::Info::new("Substates", "0.1.0"))
        .build()
}

mod substate {
    use super::*;

    routes! {
        state = super::AppState,
        prefix = "/sub",
        "/" => { get(value) },
    }

    #[handler]
    pub async fn value(State(db): State<super::Db>) -> String {
        db.0.to_string()
    }
}

#[tokio::test]
async fn a_table_state_extracts_a_substate() {
    let built = app(&substate::MODULE);
    let response = built.oneshot_get("/sub").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&*body, b"db");
}

mod nested {
    use super::*;

    routes! {
        state = super::AppState,
        prefix = "/nested",
        "/admin" [tags = ["admin"]] {
            "/" => { get(value) },
        },
    }

    #[handler]
    pub async fn value(State(db): State<super::Db>) -> String {
        db.0.to_string()
    }
}

#[tokio::test]
async fn an_inline_group_inherits_the_table_state() {
    let built = app(&nested::MODULE);
    let response = built.oneshot_get("/nested/admin").await;
    assert_eq!(response.status(), StatusCode::OK);
}

mod secured {
    use super::*;

    routes! {
        state = super::AppState,
        prefix = "/secured",
        security = [Bearer],
        "/" => { get(value) },
    }

    #[handler]
    pub async fn value(State(db): State<super::Db>) -> String {
        db.0.to_string()
    }
}

#[tokio::test]
async fn a_secured_route_can_extract_a_substate() {
    let built = app(&secured::MODULE);
    let request = Request::builder()
        .uri("/secured")
        .header(header::AUTHORIZATION, "Bearer ok")
        .body(Body::empty())
        .unwrap();
    let response = built.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = built.oneshot_get("/secured").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

mod custom_extractor {
    use super::*;

    routes! {
        state = super::AppState,
        prefix = "/custom",
        "/" => { get(value) },
    }

    #[handler]
    pub async fn value(db: super::DbExtractor) -> String {
        db.0.0.to_string()
    }
}

#[tokio::test]
async fn a_table_state_supports_a_custom_stateful_extractor_without_a_state_argument() {
    let built = app(&custom_extractor::MODULE);
    let response = built.oneshot_get("/custom").await;
    assert_eq!(response.status(), StatusCode::OK);
}

mod spread_child {
    use super::*;

    routes! {
        state = super::AppState,
        prefix = "/spread",
        "/" => { get(value) },
    }

    #[handler]
    pub async fn value(State(db): State<super::Db>) -> String {
        db.0.to_string()
    }
}

mod spread_root {
    use super::*;

    routes! {
        ...super::spread_child,
    }
}

#[tokio::test]
async fn a_spread_module_keeps_its_own_state() {
    let built = app(&spread_root::MODULE);
    let response = built.oneshot_get("/spread").await;
    assert_eq!(response.status(), StatusCode::OK);
}

mod wrong_state {
    use super::*;

    routes! {
        state = super::OtherState,
        prefix = "/wrong",
        "/" => { get(value) },
    }

    #[handler]
    pub async fn value() -> &'static str {
        "wrong"
    }
}

#[test]
fn a_table_state_that_differs_from_the_app_state_is_a_build_error() {
    let error = App::new(AppState::default())
        .include(&wrong_state::MODULE)
        .try_build()
        .err()
        .expect("a different table state must not build");
    assert!(matches!(error, BuildError::StateMismatch { .. }));
}
