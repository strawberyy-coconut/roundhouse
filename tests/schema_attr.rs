//! `#[roundhouse::schema]` is the one-annotation form of `ToSchema` + `ApiSchema`.

use roundhouse::interop::utoipa::openapi::schema::Components;
use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

#[roundhouse::schema]
#[derive(Serialize, Deserialize)]
#[schema(description = "A stored thing.")]
pub struct Thing {
    #[schema(min_length = 1)]
    pub name: String,
}

#[roundhouse::schema]
#[derive(Serialize)]
pub enum Shape {
    Circle { radius: f64 },
    Square { side: f64 },
}

#[test]
fn the_attribute_derives_both_traits() {
    fn assert_both<T: roundhouse::interop::ToSchema + roundhouse::ApiSchema>() {}
    assert_both::<Thing>();
    assert_both::<Shape>();

    let mut components = Components::new();
    Thing::register(&mut components);
    assert!(components.schemas.contains_key("Thing"));
    assert_eq!(
        <Thing as roundhouse::ApiSchema>::description(),
        Some("A stored thing.")
    );
}

#[roundhouse::schema]
#[derive(Serialize)]
#[roundhouse(name = "schema_attr.Renamed", description = "A renamed component.")]
pub struct Renamed {
    #[roundhouse(min_length = 2, max_length = 4)]
    pub code: String,
}

#[test]
fn roundhouse_attributes_feed_the_document_and_validation() {
    let mut components = Components::new();
    Renamed::register(&mut components);
    let schema = components
        .schemas
        .get("schema_attr.Renamed")
        .expect("the explicit component name is used");
    let value = serde_json::to_value(schema).unwrap();
    assert_eq!(value["properties"]["code"]["minLength"], 2);
    assert_eq!(value["properties"]["code"]["maxLength"], 4);
}
