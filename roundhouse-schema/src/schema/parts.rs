use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};

use super::PartsValue;

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

macro_rules! parts_value_via_serde {
    ($($ty:ty),* $(,)?) => {
        $(
            impl PartsValue for $ty {
                fn parts_value(&self) -> serde_json::Value {
                    serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
                }
            }
        )*
    };
}

parts_value_via_serde!(
    bool, char, String, u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
);

impl<K, V> PartsValue for std::collections::HashMap<K, V>
where
    K: serde::Serialize + Eq + std::hash::Hash,
    V: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl<K, V> PartsValue for std::collections::BTreeMap<K, V>
where
    K: serde::Serialize + Ord,
    V: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl PartsValue for serde_json::Value {
    fn parts_value(&self) -> serde_json::Value {
        self.clone()
    }
}

impl<T> PartsValue for Vec<T>
where
    T: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(feature = "uuid")]
impl PartsValue for uuid::Uuid {
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(feature = "chrono")]
impl<Tz> PartsValue for chrono::DateTime<Tz>
where
    Tz: chrono::TimeZone,
    chrono::DateTime<Tz>: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(feature = "time")]
impl PartsValue for time::OffsetDateTime {
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(feature = "decimal")]
impl PartsValue for rust_decimal::Decimal {
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(feature = "jiff")]
impl PartsValue for jiff::Timestamp {
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
