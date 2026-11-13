//! Two Rust types must not silently share one component name.

use roundhouse::interop::ToSchema;
use roundhouse::prelude::*;
use roundhouse::{App, BuildError};

#[derive(Clone, Default)]
pub struct State;

mod first {
    use super::*;

    routes! {
        prefix = "/first",
        "/" => { get(show) },
    }

    #[derive(serde::Serialize, ToSchema, ApiSchema)]
    pub struct Page {
        pub first: String,
    }

    #[handler]
    pub async fn show() -> Json<Page> {
        Json(Page {
            first: "first".to_string(),
        })
    }
}

mod second {
    use super::*;

    routes! {
        prefix = "/second",
        "/" => { get(show) },
    }

    #[derive(serde::Serialize, ToSchema, ApiSchema)]
    pub struct Page {
        pub second: String,
    }

    #[handler]
    pub async fn show() -> Json<Page> {
        Json(Page {
            second: "second".to_string(),
        })
    }
}

#[test]
fn two_types_with_one_component_name_are_a_build_error() {
    let error = App::new(State)
        .include(&first::MODULE)
        .include(&second::MODULE)
        .try_build()
        .err()
        .expect("a duplicate component must not build");
    let BuildError::ComponentCollision {
        ref name,
        first_type,
        second_type,
        ..
    } = error
    else {
        panic!("expected ComponentCollision, got {error}");
    };
    assert_eq!(name, "Page");
    assert!(first_type.contains("Page"));
    assert!(second_type.contains("Page"));
    assert_ne!(first_type, second_type);
    assert!(error.to_string().contains("#[schema(as = module::Name)]"));
}

mod renamed_first {
    use super::*;

    routes! {
        prefix = "/renamed-first",
        "/" => { get(show) },
    }

    #[derive(serde::Serialize, ToSchema, ApiSchema)]
    #[schema(as = first::Page)]
    pub struct Page {
        pub first: String,
    }

    #[handler]
    pub async fn show() -> Json<Page> {
        Json(Page {
            first: "first".to_string(),
        })
    }
}

mod renamed_second {
    use super::*;

    routes! {
        prefix = "/renamed-second",
        "/" => { get(show) },
    }

    #[derive(serde::Serialize, ToSchema, ApiSchema)]
    #[schema(as = second::Page)]
    pub struct Page {
        pub second: String,
    }

    #[handler]
    pub async fn show() -> Json<Page> {
        Json(Page {
            second: "second".to_string(),
        })
    }
}

#[test]
fn schema_as_disambiguates_component_names() {
    let built = App::new(State)
        .include(&renamed_first::MODULE)
        .include(&renamed_second::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Components", "0.1.0"))
        .try_build()
        .expect("renamed components build");
    let document = serde_json::to_value(built.document()).unwrap();
    assert!(document["components"]["schemas"]["first.Page"].is_object());
    assert!(document["components"]["schemas"]["second.Page"].is_object());
}

mod reused {
    use super::*;

    routes! {
        prefix = "/reused",
        "/one" => { get(one) },
        "/two" => { get(two) },
    }

    #[derive(serde::Serialize, ToSchema, ApiSchema)]
    pub struct Thing {
        pub id: u64,
    }

    #[handler]
    pub async fn one() -> Json<Thing> {
        Json(Thing { id: 1 })
    }

    #[handler]
    pub async fn two() -> Json<Thing> {
        Json(Thing { id: 2 })
    }
}

#[test]
fn the_same_type_on_two_routes_is_registered_once() {
    App::new(State)
        .include(&reused::MODULE)
        .try_build()
        .expect("the same type may appear on several routes");
}

roundhouse::app_state!(State);
