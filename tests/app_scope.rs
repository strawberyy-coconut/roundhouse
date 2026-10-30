//! `app!(state, module)` roots the app at a fragment from another module.

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

mod routes {
    use super::*;

    routes! {
        prefix = "/app-scope",
        get("/x", x),
    }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

mod launch {
    use super::*;

    pub fn app() -> roundhouse::App<State> {
        roundhouse::app!(State, crate::routes)
    }
}

#[tokio::test]
async fn app_macro_finds_routes_from_a_submodule() {
    let built = launch::app().build();
    assert_eq!(
        built.oneshot_get("/app-scope/x").await.status(),
        axum::http::StatusCode::OK
    );
}
