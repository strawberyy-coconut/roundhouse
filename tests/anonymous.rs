//! `anonymous` clears inherited security guards for one route or subtree.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

roundhouse::app_state!(State);

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
        _state: &State,
    ) -> Result<(), Problem> {
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

    routes! {
        prefix = "/guarded",
        security = [Bearer],

        "/open" => {
            get(open, anonymous),
        },

        "/private" => {
            get(private),
        },

        prefix "/public" [anonymous] {
            "/ok" => {
                get(public_ok),
            },
        },
    }

    #[handler]
    pub async fn open() -> &'static str {
        "open"
    }

    #[handler]
    pub async fn private() -> &'static str {
        "private"
    }

    #[handler]
    pub async fn public_ok() -> &'static str {
        "public"
    }
}

fn built() -> roundhouse::Built {
    roundhouse::App::new(State)
        .include(&guarded::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Anonymous", "0.1.0"))
        .build()
}

async fn get(app: &roundhouse::Built, uri: &str, token: Option<&str>) -> axum::response::Response {
    let mut builder = Request::builder().uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, token);
    }
    app.oneshot(builder.body(Body::empty()).unwrap()).await
}

#[tokio::test]
async fn an_anonymous_entry_skips_the_module_guard() {
    let app = built();
    assert_eq!(
        get(&app, "/guarded/open", None).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        get(&app, "/guarded/private", None).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, "/guarded/private", Some("Bearer ok"))
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn an_anonymous_group_skips_the_parent_guard() {
    let app = built();
    assert_eq!(
        get(&app, "/guarded/public/ok", None).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_document_opts_anonymous_operations_out_of_security() {
    let app = built();
    let document = serde_json::to_value(app.document()).unwrap();

    assert_eq!(
        document["paths"]["/guarded/open"]["get"]["security"],
        serde_json::json!([])
    );
    assert_eq!(
        document["paths"]["/guarded/public/ok"]["get"]["security"],
        serde_json::json!([])
    );
    assert_eq!(
        document["paths"]["/guarded/private"]["get"]["security"][0]["bearer"],
        serde_json::json!([])
    );
}
