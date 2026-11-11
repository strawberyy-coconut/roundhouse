//! `#[roundhouse::schema]` is the one-annotation form of `ToSchema` + `ApiSchema`.

use roundhouse::prelude::*;
use roundhouse::utoipa::openapi::schema::Components;
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
    fn assert_both<T: roundhouse::ToSchema + roundhouse::ApiSchema>() {}
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
