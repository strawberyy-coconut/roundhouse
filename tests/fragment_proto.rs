//! Prototype: one `routes!` macro, module spreads, and a fragment-aware `app!`.

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

mod api {
    use super::*;

    mount!("/api");

    routes! {
        get("/", root, operation_id = "fragment_api_root"),
        ...blog
    }

    #[handler]
    pub async fn root() -> &'static str {
        "api"
    }

    pub mod blog {
        use super::*;

        mount!("/blog");

        routes! { get("/", list, operation_id = "fragment_blog_list") }

        #[handler]
        pub async fn list() -> &'static str {
            "blog"
        }
    }
}

// The root table composes the module's fragment. `app!(.., fragment)` calls the generated
// `__roundhouse_fragment` in this module.
routes! {
    ...api
}

#[tokio::test]
async fn one_macro_composes_modules_and_app_finds_the_fragment() {
    let built = roundhouse::app!(State, fragment)
        .openapi(|| roundhouse::openapi::Info::new("Fragments", "0.1.0"))
        .build();

    assert_eq!(
        built.oneshot_get("/api").await.status(),
        axum::http::StatusCode::OK
    );
    assert_eq!(
        built.oneshot_get("/api/blog").await.status(),
        axum::http::StatusCode::OK
    );

    let document = serde_json::to_value(built.document()).unwrap();
    assert!(document["paths"]["/api"]["get"].is_object());
    assert!(document["paths"]["/api/blog"]["get"].is_object());
}
