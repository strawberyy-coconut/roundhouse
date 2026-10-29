//! `app!` scopes by crate, so moving the call into a submodule cannot empty the app.

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

mod routes {
    use super::*;

    mount!("/app-scope");

    routes! { get("/x", x) }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

mod launch {
    use super::*;

    pub fn app() -> roundhouse::App<State> {
        roundhouse::app!(State)
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
