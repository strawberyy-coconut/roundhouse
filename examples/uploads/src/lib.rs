pub mod routes;

#[derive(Clone, Default)]
pub struct AppState;

use roundhouse::openapi::Info;

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state)
        .max_body(2 * 1024 * 1024)
        .openapi(|| Info::new("Uploads API", env!("CARGO_PKG_VERSION")))
        .openapi_route("/spec.json")
        .docs_route("/reference")
}

/// The same declarations as [`app`], resolved without constructing `AppState`.
pub fn document() -> roundhouse::Document {
    roundhouse::Document::new()
        .scope(module_path!())
        .openapi(|| Info::new("Uploads API", env!("CARGO_PKG_VERSION")))
}
