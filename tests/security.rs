//! A `security` declaration is enforced by the router, not merely documented.
//!
//! `mount!(security = [A])` and a `routes!` entry's `security = [A]` are scheme *types*: the
//! guard is monomorphized over `A` and installed on every route it covers. These tests check the
//! runtime rejection, the published requirement, and the build-time state check.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use roundhouse::prelude::*;
use roundhouse::{App, BuildError, Built};

#[derive(Clone, Default)]
pub struct State {
    calls: Arc<AtomicUsize>,
}

impl State {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

pub struct Bearer;

impl AuthScheme for Bearer {
    type State = State;
    type Identity = ();

    const NAME: &'static str = "bearer";
    const KIND: roundhouse::SecuritySchemeKind = roundhouse::SecuritySchemeKind::HttpBearer;
}

impl Authenticate for Bearer {
    async fn authenticate(
        parts: &mut axum::http::request::Parts,
        state: &State,
    ) -> Result<(), Problem> {
        state.calls.fetch_add(1, Ordering::SeqCst);
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

    fn presents_credentials(parts: &axum::http::request::Parts) -> bool {
        parts.headers.contains_key(header::AUTHORIZATION)
    }
}

mod guarded {
    use super::*;

    mount!("/guarded", security = [Bearer]);

    routes! {
        get("/open", open),
        get("/own", own),
        get("/optional", optional),
    }

    #[handler]
    pub async fn open() -> &'static str {
        "open"
    }

    #[handler]
    pub async fn own(_auth: Auth<Bearer>) -> &'static str {
        "own"
    }

    #[handler]
    pub async fn optional(_auth: Option<Auth<Bearer>>) -> &'static str {
        "optional"
    }
}

mod entry {
    use super::*;

    routes! {
        get("/entry", entry, security = [Bearer]),
    }

    #[handler]
    pub async fn entry() -> &'static str {
        "entry"
    }
}

mod mismatched {
    use super::*;

    #[derive(Clone)]
    pub struct OtherState;

    pub struct Other;

    impl AuthScheme for Other {
        type State = OtherState;
        type Identity = ();

        const NAME: &'static str = "other";
        const KIND: roundhouse::SecuritySchemeKind = roundhouse::SecuritySchemeKind::HttpBearer;
    }

    impl Authenticate for Other {
        async fn authenticate(
            _parts: &mut axum::http::request::Parts,
            _state: &OtherState,
        ) -> Result<(), Problem> {
            Ok(())
        }

        fn presents_credentials(_parts: &axum::http::request::Parts) -> bool {
            true
        }
    }

    mount!("/mismatched", security = [Other]);

    routes! {
        get("/x", x),
    }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

fn scoped(module: &'static str, state: State) -> App<State> {
    App::new(state)
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Security", "0.1.0"))
}

fn built() -> (Built, State) {
    let state = State::default();
    let app = scoped(concat!(module_path!(), "::guarded"), state.clone()).build();
    (app, state)
}

async fn request(app: &Built, uri: &str, authorization: Option<&str>) -> axum::response::Response {
    let mut builder = Request::builder().uri(uri);
    if let Some(value) = authorization {
        builder = builder.header(header::AUTHORIZATION, value);
    }
    app.oneshot(builder.body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn mount_security_is_enforced_and_documented() {
    let (app, _) = built();
    let document = serde_json::to_value(app.document()).unwrap();
    assert!(
        document["paths"]["/guarded/open"]["get"]["security"][0]["bearer"].is_array(),
        "the mount guard must publish its requirement"
    );
    assert!(
        document["paths"]["/guarded/own"]["get"]["security"][0]["bearer"].is_array(),
        "a handler's own extractor publishes the same requirement"
    );
    assert_eq!(
        document["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );

    let response = request(&app, "/guarded/open", None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
    assert_eq!(
        request(&app, "/guarded/open", Some("Bearer ok"))
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn a_handler_that_already_enforces_the_scheme_is_not_authenticated_twice() {
    let (app, state) = built();

    let response = request(&app, "/guarded/own", Some("Bearer ok")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        state.calls(),
        1,
        "the mount guard must defer to Auth<Bearer>"
    );

    let response = request(&app, "/guarded/open", Some("Bearer ok")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(state.calls(), 2, "the mount guard runs for a bare route");
}

#[tokio::test]
async fn an_optional_handler_still_requires_the_mount_scheme() {
    let (app, _) = built();

    assert_eq!(
        request(&app, "/guarded/optional", None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "/guarded/optional", Some("Bearer ok"))
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn entry_security_is_enforced_too() {
    let app = scoped(concat!(module_path!(), "::entry"), State::default()).build();

    assert_eq!(
        request(&app, "/entry", None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "/entry", Some("Bearer ok")).await.status(),
        StatusCode::OK
    );
}

#[test]
fn a_guard_against_another_state_is_a_build_error() {
    let error = scoped(concat!(module_path!(), "::mismatched"), State::default())
        .try_build()
        .err()
        .expect("the guard's state must match the app state");
    assert!(matches!(
        error,
        BuildError::SecurityStateMismatch { ref name, .. } if name == "other"
    ));
    assert!(error.to_string().contains("OtherState"));
}
