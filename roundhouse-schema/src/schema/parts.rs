use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};

pub fn object_from_params(params: Vec<crate::Param>) -> RefOr<Schema> {
    let mut builder = ObjectBuilder::new().schema_type(Type::Object);
    for param in params {
        if param.required {
            builder = builder.required(param.name.clone());
        }
        builder = builder.property(param.name, param.schema);
    }
    builder.into()
}
