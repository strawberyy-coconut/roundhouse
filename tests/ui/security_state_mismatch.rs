use axum::http::request::Parts;
use roundhouse::prelude::*;
use roundhouse::{Problem, SecuritySchemeKind};

#[derive(Clone, Default)]
struct SchemeState;

#[derive(Clone, Default)]
struct HandlerState;

struct Bearer;

impl AuthScheme for Bearer {
    type State = SchemeState;
    type Identity = ();

    const NAME: &'static str = "Bearer";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::HttpBearer;
}

impl Authenticate for Bearer {
    async fn authenticate(_parts: &mut Parts, _state: &SchemeState) -> Result<(), Problem> {
        Ok(())
    }

    fn presents_credentials(_parts: &Parts) -> bool {
        true
    }
}

mount!("/x");

routes! { get("/", show, security = [Bearer]) }

#[handler]
async fn show(State(_state): State<HandlerState>) -> &'static str {
    "ok"
}

fn main() {}
