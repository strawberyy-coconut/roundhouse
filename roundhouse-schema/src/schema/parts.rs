use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};

use super::Params;

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

impl Params for serde_json::Value {
    fn parts_value(&self) -> serde_json::Value {
        self.clone()
    }
}

impl<K, V> Params for std::collections::HashMap<K, V>
where
    K: serde::Serialize + Eq + std::hash::Hash,
    V: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl<K, V> Params for std::collections::BTreeMap<K, V>
where
    K: serde::Serialize + Ord,
    V: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl<T> Params for Vec<T>
where
    T: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
