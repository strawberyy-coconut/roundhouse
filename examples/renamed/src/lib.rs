//! A renamed `roundhouse` dependency (`rh = { package = "roundhouse", .. }`).
//!
//! Every macro expansion resolves the crate name in the *expanding* crate, so this compiles
//! without a single `::roundhouse` path.

use rh::prelude::*;

#[derive(Clone, Default)]
pub struct AppState;

pub mod routes {
    use super::*;

    mount!("/renamed");

    routes! { get("/x", x, operation_id = "renamed_x") }

    #[derive(serde::Serialize, ApiSchema)]
    pub struct Out {
        pub value: i64,
    }

    #[handler]
    pub async fn x() -> Json<Out> {
        Json(Out { value: 1 })
    }
}

pub fn app() -> rh::App<AppState> {
    rh::App::new(AppState).scope(module_path!())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_renamed_dependency_builds() {
        let built = super::app().build();
        assert_eq!(built.routes().len(), 1);
        assert_eq!(built.routes()[0].path, "/renamed/x");
    }
}
