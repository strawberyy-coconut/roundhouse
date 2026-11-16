pub mod routes;

#[derive(Clone, Default)]
pub struct AppState;

roundhouse::app_state!(AppState);

roundhouse::api! {
    state = AppState,
    title = "Uploads API",
    version = env!("CARGO_PKG_VERSION"),
    root = routes::MODULE,
}

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state, routes)
        .max_body(2 * 1024 * 1024)
        .contract(roundhouse::include_contract!())
        .openapi_route("/spec.json")
        .docs_route("/reference")
}
