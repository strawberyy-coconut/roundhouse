//! Route-level policy applied by the generated shim.
//!
//! Validation is on by default; `validate = false` opts out. A route's single security scheme is
//! enforced as an extractor, without a handler argument when the handler does not ask for one.
//! The handler functions themselves are never rewritten.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header, request::Parts};
use roundhouse::prelude::*;
use roundhouse::{App, Built, Problem, SecuritySchemeKind, Validation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct AuthState {
    pub calls: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, ApiSchema)]
pub struct NewNote {
    #[schema(min_length = 3)]
    pub title: String,
}

#[derive(Debug, Deserialize, PathParams)]
pub struct ItemPath {
    #[schema(minimum = 1)]
    pub id: i64,
}

#[derive(Debug, Deserialize, QueryParams)]
pub struct Filter {
    #[schema(min_length = 2)]
    pub q: Option<String>,
}

pub struct Counter;

impl AuthScheme for Counter {
    type State = AuthState;
    type Identity = ();

    const NAME: &'static str = "Counter";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::HttpBearer;
}

impl Authenticate for Counter {
    async fn authenticate(parts: &mut Parts, state: &AuthState) -> Result<Self::Identity, Problem> {
        state.calls.fetch_add(1, Ordering::SeqCst);
        if parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            == Some("Bearer ok")
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

fn scoped(module: &'static str) -> Built {
    App::new(AuthState::default())
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Shim", "0.1.0"))
        .build()
}

fn scoped_with_counter(module: &'static str) -> (Built, Arc<AtomicUsize>) {
    let state = AuthState::default();
    let calls = state.calls.clone();
    let built = App::new(state)
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Shim", "0.1.0"))
        .build();
    (built, calls)
}

fn authed_get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(http::Method::GET)
        .uri(uri)
        .header(header::AUTHORIZATION, "Bearer ok")
        .body(Body::empty())
        .expect("valid request")
}

async fn problem_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body collects");
    serde_json::from_slice(&bytes).expect("problem is JSON")
}

mod json_body {
    use super::*;

    mount!("/json");

    routes! { post("/", create, operation_id = "shim_json_create") }

    #[handler]
    pub async fn create(axum::Json(body): axum::Json<NewNote>) -> axum::Json<NewNote> {
        axum::Json(body)
    }
}

mod json_opt_out {
    use super::*;

    mount!("/json-opt-out");

    routes! {
        post("/", create, operation_id = "shim_json_opt_out_create", validate = false)
    }

    #[handler]
    pub async fn create(axum::Json(body): axum::Json<NewNote>) -> axum::Json<NewNote> {
        axum::Json(body)
    }
}

mod owned_json {
    use super::*;

    mount!("/owned-json");

    routes! { post("/", create, operation_id = "shim_owned_json_create") }

    #[handler]
    pub async fn create(Json(body): Json<NewNote>) -> Json<NewNote> {
        Json(body)
    }
}

mod path_param {
    use super::*;

    mount!("/path");

    routes! { get("/{id}", show, operation_id = "shim_path_show") }

    #[handler]
    pub async fn show(axum::extract::Path(path): axum::extract::Path<ItemPath>) -> String {
        path.id.to_string()
    }
}

mod query_param {
    use super::*;

    mount!("/query");

    routes! { get("/", list, operation_id = "shim_query_list") }

    #[handler]
    pub async fn list(axum::extract::Query(filter): axum::extract::Query<Filter>) -> String {
        filter.q.unwrap_or_default()
    }
}

mod auth_with_handler {
    use super::*;

    mount!("/auth");

    routes! { get("/", whoami, operation_id = "shim_auth_whoami", security = [Counter]) }

    #[handler]
    pub async fn whoami(_auth: Auth<Counter>) -> &'static str {
        "ok"
    }
}

mod auth_without_handler {
    use super::*;

    mount!("/auth-open");

    routes! { get("/", open, operation_id = "shim_auth_open", security = [Counter]) }

    #[handler]
    pub async fn open() -> &'static str {
        "ok"
    }
}

mod auth_with_state {
    use super::*;

    mount!("/auth-state");

    routes! { get("/", calls, operation_id = "shim_auth_calls", security = [Counter]) }

    #[handler]
    pub async fn calls(State(state): State<AuthState>, _auth: Auth<Counter>) -> String {
        state.calls.load(Ordering::SeqCst).to_string()
    }
}

mod auth_and_body {
    use super::*;

    mount!("/auth-body");

    routes! { post("/", create, operation_id = "shim_auth_create", security = [Counter]) }

    #[handler]
    pub async fn create(_auth: Auth<Counter>, axum::Json(body): axum::Json<NewNote>) -> String {
        body.title
    }
}

mod strict_validated {
    use super::*;

    mount!("/strict-validated");

    routes! { post("/", create, operation_id = "shim_strict_validated") }

    #[handler]
    pub async fn create(axum::Json(_body): axum::Json<NewNote>) -> StatusCode {
        StatusCode::CREATED
    }
}

mod strict_unvalidated {
    use super::*;

    mount!("/strict-unvalidated");

    routes! {
        post("/", create, operation_id = "shim_strict_unvalidated", validate = false)
    }

    #[handler]
    pub async fn create(axum::Json(_body): axum::Json<NewNote>) -> StatusCode {
        StatusCode::CREATED
    }
}

mod strict_response {
    use super::*;

    mount!("/strict-response");

    routes! { get("/", read, operation_id = "shim_strict_response") }

    #[handler]
    pub async fn read() -> axum::Json<NewNote> {
        axum::Json(NewNote {
            title: "abc".to_string(),
        })
    }
}

mod strict_json {
    use super::*;

    mount!("/strict-json");

    routes! { post("/", create, operation_id = "shim_strict_json_create") }

    #[handler]
    pub async fn create(Strict(axum::Json(_body)): Strict<axum::Json<NewNote>>) -> StatusCode {
        StatusCode::NO_CONTENT
    }
}

mod strict_path {
    use super::*;

    mount!("/strict-path");

    routes! { get("/{id}", show, operation_id = "shim_strict_path_show") }

    #[handler]
    pub async fn show(
        Strict(axum::extract::Path(path)): Strict<axum::extract::Path<ItemPath>>,
    ) -> String {
        path.id.to_string()
    }
}

mod result_body {
    use super::*;

    mount!("/result");

    routes! { post("/", create, operation_id = "shim_result_create") }

    #[handler]
    pub async fn create(
        body: Result<axum::Json<NewNote>, axum::extract::rejection::JsonRejection>,
    ) -> Result<axum::Json<NewNote>, axum::extract::rejection::JsonRejection> {
        body
    }
}

#[test]
fn the_body_probe_recognises_a_foreign_json_body() {
    fn assert_api<T: roundhouse::doc::ApiSchema>() {}
    assert_api::<NewNote>();

    use roundhouse::shim::{BodyProbe, ProbeBody};
    assert!(
        (&BodyProbe::<axum::Json<NewNote>>::new()).probe_body(),
        "the probe resolves at the concrete type"
    );
    assert!(
        !(&BodyProbe::<axum::Json<NotASchema>>::new()).probe_body(),
        "a body without ApiSchema cannot be validated"
    );
}

#[derive(Debug, serde::Deserialize)]
pub struct NotASchema {
    pub value: String,
}

#[test]
fn a_validated_body_is_not_marked_unverified() {
    let built = scoped(concat!(module_path!(), "::json_body"));
    let document = serde_json::to_value(built.document()).unwrap();
    let operation = &document["paths"]["/json"]["post"];
    assert!(
        operation.get("x-roundhouse-unverified-body").is_none(),
        "{operation}"
    );
}

#[tokio::test]
async fn a_strict_axum_extractor_keeps_axum_and_adds_the_problem_rejection() {
    let built = scoped(concat!(module_path!(), "::strict_json"));

    let response = built
        .oneshot_json(http::Method::POST, "/strict-json", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    let problem = problem_json(response).await;
    assert_eq!(problem["errors"][0]["pointer"], "/title");

    let response = built
        .oneshot_json(http::Method::POST, "/strict-json", "{not json")
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );

    let response = built
        .oneshot_json(http::Method::POST, "/strict-json", r#"{"title":"abc"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_strict_axum_path_is_validated_by_the_engine() {
    let built = scoped(concat!(module_path!(), "::strict_path"));
    let response = built.oneshot_get("/strict-path/0").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = built.oneshot_get("/strict-path/2").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn a_strict_body_is_not_marked_unverified() {
    let built = scoped(concat!(module_path!(), "::strict_json"));
    let document = serde_json::to_value(built.document()).unwrap();
    let operation = &document["paths"]["/strict-json"]["post"];
    assert!(
        operation.get("x-roundhouse-unverified-body").is_none(),
        "{operation}"
    );
}

#[tokio::test]
async fn a_foreign_body_is_validated_by_default() {
    let built = scoped(concat!(module_path!(), "::json_body"));
    let response = built
        .oneshot_json(http::Method::POST, "/json", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let problem = problem_json(response).await;
    assert_eq!(problem["errors"][0]["pointer"], "/title");

    let response = built
        .oneshot_json(http::Method::POST, "/json", r#"{"title":"abc"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn validate_false_disables_the_shim_check() {
    let built = scoped(concat!(module_path!(), "::json_opt_out"));
    let response = built
        .oneshot_json(http::Method::POST, "/json-opt-out", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn app_level_off_disables_the_shim() {
    let built = App::new(AuthState::default())
        .scope(concat!(module_path!(), "::json_body"))
        .validation(Validation::Off)
        .openapi(|| roundhouse::openapi::Info::new("Shim", "0.1.0"))
        .build();
    let response = built
        .oneshot_json(http::Method::POST, "/json", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_frameworks_own_extractor_is_not_validated_twice() {
    let built = scoped(concat!(module_path!(), "::owned_json"));
    let response = built
        .oneshot_json(http::Method::POST, "/owned-json", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_foreign_path_extractor_is_validated() {
    let built = scoped(concat!(module_path!(), "::path_param"));
    let response = built.oneshot_get("/path/0").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = built.oneshot_get("/path/2").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_foreign_query_extractor_is_validated() {
    let built = scoped(concat!(module_path!(), "::query_param"));
    let response = built.oneshot_get("/query?q=a").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = built.oneshot_get("/query?q=ab").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_route_scheme_is_enforced_without_a_handler_argument() {
    let (built, calls) = scoped_with_counter(concat!(module_path!(), "::auth_without_handler"));

    let response = built.oneshot_get("/auth-open").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let response = built.oneshot(authed_get("/auth-open")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_handler_that_declares_auth_authenticates_once() {
    let (built, calls) = scoped_with_counter(concat!(module_path!(), "::auth_with_handler"));

    let response = built.oneshot(authed_get("/auth")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the handler's Auth and the shim's enforcement must not both authenticate"
    );

    let response = built.oneshot_get("/auth").await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_stateful_handler_shares_the_schemes_state() {
    let (built, calls) = scoped_with_counter(concat!(module_path!(), "::auth_with_state"));

    let response = built.oneshot(authed_get("/auth-state")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn auth_precedes_the_body_and_validation_still_runs() {
    let (built, _calls) = scoped_with_counter(concat!(module_path!(), "::auth_and_body"));

    let response = built
        .oneshot_json(http::Method::POST, "/auth-body", r#"{"title":"abc"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let request = Request::builder()
        .method(http::Method::POST)
        .uri("/auth-body")
        .header(header::AUTHORIZATION, "Bearer ok")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"title":"ab"}"#))
        .expect("valid request");
    let response = built.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let request = Request::builder()
        .method(http::Method::POST)
        .uri("/auth-body")
        .header(header::AUTHORIZATION, "Bearer ok")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"title":"abc"}"#))
        .expect("valid request");
    let response = built.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn a_result_typed_body_owns_its_rejection_and_is_not_validated() {
    let built = scoped(concat!(module_path!(), "::result_body"));
    let response = built
        .oneshot_json(http::Method::POST, "/result", r#"{"title":"ab"}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn the_opt_out_route_keeps_the_body_marker() {
    let built = scoped(concat!(module_path!(), "::json_opt_out"));
    let document = serde_json::to_value(built.document()).unwrap();
    let operation = &document["paths"]["/json-opt-out"]["post"];
    assert_eq!(operation["x-roundhouse-unverified-body"], true);
    assert_eq!(operation["x-roundhouse-validation"], false);
}

#[test]
fn strict_accepts_a_body_the_shim_validates() {
    App::new(AuthState::default())
        .scope(concat!(module_path!(), "::strict_validated"))
        .openapi(|| roundhouse::openapi::Info::new("Shim", "0.1.0"))
        .strict()
        .try_build()
        .expect("a validated foreign body is no longer unverified");
}

#[test]
fn strict_rejects_an_opted_out_foreign_body() {
    let error = App::new(AuthState::default())
        .scope(concat!(module_path!(), "::strict_unvalidated"))
        .openapi(|| roundhouse::openapi::Info::new("Shim", "0.1.0"))
        .strict()
        .try_build()
        .err()
        .expect("an opted-out foreign body is still unverified");
    assert!(matches!(
        error,
        roundhouse::BuildError::UnverifiedBody { .. }
    ));
}

#[test]
fn strict_rejects_a_foreign_response() {
    let error = App::new(AuthState::default())
        .scope(concat!(module_path!(), "::strict_response"))
        .openapi(|| roundhouse::openapi::Info::new("Shim", "0.1.0"))
        .strict()
        .try_build()
        .err()
        .expect("a foreign response is still unverified");
    assert!(matches!(
        error,
        roundhouse::BuildError::UnverifiedResponse { .. }
    ));
}

#[test]
fn the_document_publishes_the_route_scheme() {
    let built = scoped(concat!(module_path!(), "::auth_without_handler"));
    let document = serde_json::to_value(built.document()).unwrap();
    assert_eq!(
        document["paths"]["/auth-open"]["get"]["security"][0]["Counter"],
        serde_json::json!([])
    );
    assert!(
        document["components"]["securitySchemes"]["Counter"].is_object(),
        "{document}"
    );
}
