use std::sync::Arc;

use roundhouse::prelude::*;
use roundhouse::utoipa::openapi::schema::{ObjectBuilder, Type};
use serde::{Deserialize, Serialize};

routes! {
    post("/even", even, summary = "Validate with a custom function"),
    get("/utoipa", external, summary = "A schema delegated to utoipa"),
    any("/echo", echo, summary = "Any HTTP method"),
    get("/erased", erased, summary = "Extension instead of State"),
    #[cfg(feature = "extras")]
    get("/extras", extras, summary = "Compiled only with the extras feature"),
}

#[derive(Debug, Serialize, Deserialize, ApiSchema)]
pub struct Even {
    #[schema(validate_with = validate_even)]
    pub value: i64,
}

fn validate_even(value: &i64) -> Result<(), ::roundhouse::ValidationErrors> {
    if value % 2 == 0 {
        Ok(())
    } else {
        let mut errors = ::roundhouse::ValidationErrors::new();
        errors.push(::roundhouse::ValidationError::new(
            "",
            "even",
            "must be even",
        ));
        Err(errors)
    }
}

#[handler]
pub async fn even(Json(value): Json<Even>) -> Json<Even> {
    Json(value)
}

#[derive(Debug, Serialize, Deserialize, ApiSchema)]
#[schema(via = utoipa)]
pub struct External {
    pub name: String,
}

impl roundhouse::utoipa::PartialSchema for External {
    fn schema() -> roundhouse::utoipa::openapi::RefOr<roundhouse::utoipa::openapi::schema::Schema> {
        ObjectBuilder::new()
            .schema_type(Type::Object)
            .property("name", ::roundhouse::doc::schema::string())
            .required("name")
            .into()
    }
}

impl roundhouse::utoipa::ToSchema for External {}

#[handler]
pub async fn external() -> Json<External> {
    Json(External {
        name: "delegated".to_string(),
    })
}

#[handler]
pub async fn echo() -> &'static str {
    "echo"
}

#[handler]
pub async fn erased(Extension(value): Extension<Arc<usize>>) -> String {
    format!("{value}")
}

#[cfg(feature = "extras")]
#[handler]
pub async fn extras() -> &'static str {
    "extras enabled"
}
