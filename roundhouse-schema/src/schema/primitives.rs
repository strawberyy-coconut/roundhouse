use std::collections::HashMap;

use serde_json::Value;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, KnownFormat, ObjectBuilder, Schema, SchemaFormat, Type};

use crate::validate::ValidationErrors;

use super::{
    ApiSchema, any_value, array, bool_schema, bytes_schema, integer, map, nullable, number, string,
    unsigned_integer,
};
#[cfg(any(feature = "uuid", feature = "chrono", feature = "time", feature = "jiff"))]
use super::string_with_format;

impl ApiSchema for () {
    fn schema() -> RefOr<Schema> {
        any_value()
    }
}

macro_rules! primitive_schema {
    ($($ty:ty => $body:expr),* $(,)?) => {
        $(
            impl ApiSchema for $ty {
                fn schema() -> RefOr<Schema> {
                    $body
                }
            }
        )*
    };
}

primitive_schema! {
    bool => bool_schema(),
    String => string(),
    str => string(),
    char => string(),
    i8 => integer(KnownFormat::Int32),
    i16 => integer(KnownFormat::Int32),
    i32 => integer(KnownFormat::Int32),
    i64 => integer(KnownFormat::Int64),
    i128 => integer(KnownFormat::Int64),
    isize => integer(KnownFormat::Int64),
    u8 => unsigned_integer(KnownFormat::UInt8),
    u16 => unsigned_integer(KnownFormat::UInt16),
    u32 => unsigned_integer(KnownFormat::UInt32),
    u64 => unsigned_integer(KnownFormat::UInt64),
    u128 => unsigned_integer(KnownFormat::UInt64),
    usize => unsigned_integer(KnownFormat::UInt64),
    f32 => number(KnownFormat::Float),
    f64 => number(KnownFormat::Double),
    Value => any_value(),
}

impl ApiSchema for serde_json::Map<String, Value> {
    fn schema() -> RefOr<Schema> {
        map(any_value())
    }
}

impl ApiSchema for std::net::IpAddr {
    fn schema() -> RefOr<Schema> {
        string()
    }
}

impl ApiSchema for std::net::Ipv4Addr {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Ipv4)))
            .into()
    }
}

impl ApiSchema for std::net::Ipv6Addr {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .format(Some(SchemaFormat::KnownFormat(KnownFormat::Ipv6)))
            .into()
    }
}

#[cfg(feature = "uuid")]
impl ApiSchema for uuid::Uuid {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::Custom("uuid".to_string()))
    }
}

impl ApiSchema for bytes::Bytes {
    fn schema() -> RefOr<Schema> {
        bytes_schema()
    }
}

#[cfg(feature = "chrono")]
impl<Tz: chrono::TimeZone> ApiSchema for chrono::DateTime<Tz> {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::DateTime))
    }
}

#[cfg(feature = "chrono")]
impl ApiSchema for chrono::NaiveDate {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::Date))
    }
}

#[cfg(feature = "chrono")]
impl ApiSchema for chrono::NaiveDateTime {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::DateTime))
    }
}

#[cfg(feature = "time")]
impl ApiSchema for time::OffsetDateTime {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::DateTime))
    }
}

#[cfg(feature = "time")]
impl ApiSchema for time::PrimitiveDateTime {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::DateTime))
    }
}

#[cfg(feature = "time")]
impl ApiSchema for time::Date {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::Date))
    }
}

#[cfg(feature = "jiff")]
impl ApiSchema for jiff::Timestamp {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::DateTime))
    }
}

#[cfg(feature = "jiff")]
impl ApiSchema for jiff::Zoned {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::DateTime))
    }
}

#[cfg(feature = "jiff")]
impl ApiSchema for jiff::civil::Date {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::Date))
    }
}

#[cfg(feature = "jiff")]
impl ApiSchema for jiff::civil::DateTime {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::DateTime))
    }
}

#[cfg(feature = "jiff")]
impl ApiSchema for jiff::civil::Time {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::Time))
    }
}

#[cfg(feature = "jiff")]
impl ApiSchema for jiff::Span {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::Duration))
    }
}

#[cfg(feature = "jiff")]
impl ApiSchema for jiff::SignedDuration {
    fn schema() -> RefOr<Schema> {
        string_with_format(SchemaFormat::KnownFormat(KnownFormat::Duration))
    }
}

#[cfg(feature = "decimal")]
impl ApiSchema for rust_decimal::Decimal {
    fn schema() -> RefOr<Schema> {
        number(KnownFormat::Double)
    }
}

impl<T: ApiSchema> ApiSchema for Option<T> {
    fn schema() -> RefOr<Schema> {
        nullable(T::schema())
    }

    fn register(components: &mut Components) {
        T::register(components);
    }

    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        if let Some(value) = self {
            value.validate_at(pointer, errors);
        }
    }
}

impl<T: ApiSchema> ApiSchema for Vec<T> {
    fn schema() -> RefOr<Schema> {
        array(T::schema()).into()
    }

    fn register(components: &mut Components) {
        T::register(components);
    }

    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        for (index, item) in self.iter().enumerate() {
            item.validate_at(
                &crate::validate::pointer(pointer, &index.to_string()),
                errors,
            );
        }
    }
}

impl<T: ApiSchema> ApiSchema for [T] {
    fn schema() -> RefOr<Schema> {
        array(T::schema()).into()
    }

    fn register(components: &mut Components) {
        T::register(components);
    }

    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        for (index, item) in self.iter().enumerate() {
            item.validate_at(
                &crate::validate::pointer(pointer, &index.to_string()),
                errors,
            );
        }
    }
}

impl<T: ApiSchema, S> ApiSchema for HashMap<String, T, S> {
    fn schema() -> RefOr<Schema> {
        map(T::schema())
    }

    fn register(components: &mut Components) {
        T::register(components);
    }

    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        for (key, value) in self {
            value.validate_at(&crate::validate::pointer(pointer, key), errors);
        }
    }
}

impl<T: ApiSchema> ApiSchema for Box<T> {
    fn schema() -> RefOr<Schema> {
        T::schema()
    }

    fn register(components: &mut Components) {
        T::register(components);
    }

    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        (**self).validate_at(pointer, errors);
    }
}

impl<T: ApiSchema> ApiSchema for &T {
    fn schema() -> RefOr<Schema> {
        T::schema()
    }

    fn register(components: &mut Components) {
        T::register(components);
    }

    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        (*self).validate_at(pointer, errors)
    }
}
