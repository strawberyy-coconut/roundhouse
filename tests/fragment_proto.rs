//! Explicit module composition: one `routes!` macro, nested prefixes and module spreads.

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

mod api {
    use super::*;

    routes! {
        prefix = "/api",
        get "/" => root, operation_id = "fragment_api_root",
        ...blog,
    }

    #[handler]
    pub async fn root() -> &'static str {
        "api"
    }

    pub mod blog {
        use super::*;

        routes! {
            prefix = "/blog",
            get "/" => list, operation_id = "fragment_blog_list",
        }

        #[handler]
        pub async fn list() -> &'static str {
            "blog"
        }
    }
}

// The root table composes the module's fragment. `app!(..)` calls the generated
// `__roundhouse_fragment` in this module.
routes! {
    ...api,
    ...compact,
}

#[tokio::test]
async fn one_macro_composes_modules_and_app_finds_the_fragment() {
    let built = roundhouse::app!(State)
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

mod compact {
    use super::*;

    #[derive(serde::Serialize, ToSchema, ApiSchema)]
    pub struct Note {
        pub id: u64,
    }

    routes! {
        "/api" {
            "/notes" [tags = ["notes"]] {
                get "/" => list,
                post "/" => create, responses(201 = Note),
                get "/{id}" => show, operation_id = "compact_get_note",
            }
            "/blog" [tags = ["blog"]] {
                get "/posts" => posts,
            }
        }
    }

    #[handler]
    pub async fn list() -> Json<Vec<Note>> {
        Json(vec![])
    }

    #[handler]
    pub async fn create() -> Json<Note> {
        Json(Note { id: 1 })
    }

    #[handler]
    pub async fn show(Path(id): Path<u64>) -> Json<Note> {
        Json(Note { id })
    }

    #[handler]
    pub async fn posts() -> &'static str {
        "posts"
    }
}

#[test]
fn the_compact_nested_syntax_is_the_same_tree() {
    let built = roundhouse::app!(State)
        .openapi(|| roundhouse::openapi::Info::new("Fragments", "0.1.0"))
        .build();
    let document = serde_json::to_value(built.document()).unwrap();
    assert_eq!(
        document["paths"]["/api/notes"]["get"]["tags"],
        serde_json::json!(["notes"])
    );
    assert_eq!(
        document["paths"]["/api/blog/posts"]["get"]["tags"],
        serde_json::json!(["blog"])
    );
    assert!(document["paths"]["/api/notes/{id}"]["get"].is_object());
}
