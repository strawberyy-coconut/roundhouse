//! Deriving OpenAPI schemas from Rust types, and validating the same constraints.

use std::collections::HashMap;

use serde_json::Value;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{
    AdditionalProperties, ArrayBuilder, Components, KnownFormat, Object, ObjectBuilder,
    OneOfBuilder, Ref, Schema, SchemaFormat, SchemaType, Type,
};

use crate::validate::ValidationErrors;

/// The crate's own schema trait.
///
/// `schema()` returns the inline schema for primitive types and a `$ref` for named types;
/// `register()` fills the component table (it is a no-op for primitives). Both are called from
/// the same declaration, so the document cannot drift from the wire format.
///
/// `#[derive(ApiSchema)]` pairs with `utoipa::ToSchema`: utoipa owns the wire shape and component
/// name, while this trait adds the registration hook and the hand-written `validate_with` pass.
/// Declarative constraints are enforced by executing `ToSchema`'s schema, not by generated checks.
pub trait ApiSchema {
    fn schema() -> RefOr<Schema>;

    /// Register this type (and anything it references) under its component name.
    fn register(_components: &mut Components) {}

    /// Run the `validate_with` hooks, with a JSON Pointer base. Constraint checks come from the
    /// emitted schema; this method only carries custom logic.
    fn validate_at(&self, _pointer: &str, _errors: &mut ValidationErrors) {}

    /// Run the `validate_with` hooks for this value.
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        self.validate_at("", &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Custom validation hooks for `Path<T>` extractors. Declarative constraints are enforced by the
/// emitted parameter schema.
pub trait PathValidate {
    fn validate_at(&self, _pointer: &str, _errors: &mut ValidationErrors) {}

    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = ValidationErrors::new();
        self.validate_at("", &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// A path or query type's declarative schema.
///
/// The derives build this from the same parameter table they publish, so the schema the engine
/// executes is the schema the document shows. Body types use `ToSchema` instead.
pub trait PartsSchema {
    fn parts_schema(components: &mut Components) -> RefOr<Schema>;
}

/// A path or query value as JSON, for the schema engine.
///
/// The derive implements this field by field, so a parts type does not need to derive `Serialize`
/// itself. Scalar `Path<T>` types use the implementations below.
pub trait PartsValue {
    fn parts_value(&self) -> serde_json::Value;
}

/// Build the object schema for a parts type from its documented parameters.
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

/// Path-template parameter names declared by a `Path<T>` extractor.
pub trait PathNames {
    const NAMES: &'static [&'static str] = &[];

    /// The schema of a scalar `Path<T>` whose name can only come from the template.
    fn scalar_schema() -> Option<RefOr<Schema>> {
        None
    }

    fn params() -> Vec<super::Param> {
        Vec::new()
    }
}

pub fn any_value() -> RefOr<Schema> {
    Object::with_type(SchemaType::AnyValue).into()
}

pub fn string() -> RefOr<Schema> {
    Object::with_type(Type::String).into()
}

pub fn string_with_format(format: SchemaFormat) -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .format(Some(format))
        .into()
}

pub fn bytes_schema() -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .format(Some(SchemaFormat::KnownFormat(KnownFormat::Byte)))
        .into()
}

pub fn bool_schema() -> RefOr<Schema> {
    Object::with_type(Type::Boolean).into()
}

pub fn integer(format: KnownFormat) -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(Type::Integer)
        .format(Some(SchemaFormat::KnownFormat(format)))
        .into()
}

pub fn unsigned_integer(format: KnownFormat) -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(Type::Integer)
        .format(Some(SchemaFormat::KnownFormat(format)))
        .minimum(Some(0u64))
        .into()
}

pub fn number(format: KnownFormat) -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(Type::Number)
        .format(Some(SchemaFormat::KnownFormat(format)))
        .into()
}

pub fn object() -> ObjectBuilder {
    ObjectBuilder::new().schema_type(Type::Object)
}

pub fn r#ref(name: impl Into<String>) -> RefOr<Schema> {
    Ref::from_schema_name(name).into()
}

pub fn array(items: RefOr<Schema>) -> ArrayBuilder {
    ArrayBuilder::new().items(items)
}

pub fn map(value: RefOr<Schema>) -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(Type::Object)
        .additional_properties(Some(AdditionalProperties::RefOr(value)))
        .into()
}

/// OpenAPI 3.1 nullable: merge `null` into the type when possible, otherwise `oneOf`.
pub fn nullable(schema: RefOr<Schema>) -> RefOr<Schema> {
    match schema {
        RefOr::T(Schema::Object(mut object)) => {
            object.schema_type = match object.schema_type {
                SchemaType::Type(ty) => SchemaType::Array(vec![ty, Type::Null]),
                SchemaType::Array(mut types) => {
                    if !types.contains(&Type::Null) {
                        types.push(Type::Null);
                    }
                    SchemaType::Array(types)
                }
                SchemaType::AnyValue => SchemaType::Array(vec![Type::Null]),
            };
            RefOr::T(Schema::Object(object))
        }
        RefOr::T(Schema::Array(mut array)) => {
            if let SchemaType::Type(ty) = array.schema_type {
                array.schema_type = SchemaType::Array(vec![ty, Type::Null]);
            }
            RefOr::T(Schema::Array(array))
        }
        other => OneOfBuilder::new()
            .item(other)
            .item(Object::with_type(Type::Null))
            .into(),
    }
}

pub fn one_of(items: impl IntoIterator<Item = RefOr<Schema>>) -> RefOr<Schema> {
    let mut builder = OneOfBuilder::new();
    for item in items {
        builder = builder.item(item);
    }
    builder.into()
}

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

/// `PathNames` for scalar path params.
///
/// The type carries its schema but not a name: `routes!` fills the template's capture names in,
/// so `Path<u64>` produces a documented integer parameter instead of an invalid document.
macro_rules! scalar_path_names {
    ($($ty:ty),* $(,)?) => {
        $(
            impl PathNames for $ty {
                fn scalar_schema() -> Option<RefOr<Schema>> {
                    Some(<$ty as ApiSchema>::schema())
                }
            }
            impl PathValidate for $ty {}
        )*
    };
}

scalar_path_names!(
    bool, char, String, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

#[cfg(feature = "uuid")]
scalar_path_names!(uuid::Uuid);

#[cfg(feature = "chrono")]
scalar_path_names!(
    chrono::NaiveDate,
    chrono::NaiveDateTime,
    chrono::DateTime<chrono::Utc>,
);

#[cfg(feature = "time")]
scalar_path_names!(time::Date, time::OffsetDateTime, time::PrimitiveDateTime,);

#[cfg(feature = "jiff")]
scalar_path_names!(
    jiff::Timestamp,
    jiff::Zoned,
    jiff::civil::Date,
    jiff::civil::DateTime,
    jiff::civil::Time,
);

#[cfg(feature = "decimal")]
scalar_path_names!(rust_decimal::Decimal);

impl<K, V, S> PathNames for HashMap<K, V, S> {}
impl<K, V, S> PathValidate for HashMap<K, V, S> {}

impl<K, V> PathNames for std::collections::BTreeMap<K, V> {}
impl<K, V> PathValidate for std::collections::BTreeMap<K, V> {}

macro_rules! tuple_path_names {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> PathNames for ($($name,)+) {}
        impl<$($name),+> PathValidate for ($($name,)+) {}
    };
}

tuple_path_names!(A, B);
tuple_path_names!(A, B, C);
tuple_path_names!(A, B, C, D);
tuple_path_names!(A, B, C, D, E);
tuple_path_names!(A, B, C, D, E, F);
tuple_path_names!(A, B, C, D, E, F, G);
tuple_path_names!(A, B, C, D, E, F, G, H);

impl<T: PathValidate> PathValidate for Option<T> {
    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        if let Some(value) = self {
            value.validate_at(pointer, errors);
        }
    }
}

impl<T: PathNames> PathNames for Option<T> {
    const NAMES: &'static [&'static str] = T::NAMES;

    fn scalar_schema() -> Option<RefOr<Schema>> {
        T::scalar_schema()
    }

    fn params() -> Vec<super::Param> {
        let mut params = T::params();
        for param in &mut params {
            param.required = false;
        }
        params
    }
}

fn map_object(schema: RefOr<Schema>, update: impl FnOnce(&mut Object)) -> RefOr<Schema> {
    match schema {
        RefOr::T(Schema::Object(mut object)) => {
            update(&mut object);
            RefOr::T(Schema::Object(object))
        }
        other => other,
    }
}

fn map_array(
    schema: RefOr<Schema>,
    update: impl FnOnce(&mut utoipa::openapi::schema::Array),
) -> RefOr<Schema> {
    match schema {
        RefOr::T(Schema::Array(mut array)) => {
            update(&mut array);
            RefOr::T(Schema::Array(array))
        }
        other => other,
    }
}

pub fn with_min_length(schema: RefOr<Schema>, value: usize) -> RefOr<Schema> {
    map_object(schema, |object| object.min_length = Some(value))
}

pub fn with_max_length(schema: RefOr<Schema>, value: usize) -> RefOr<Schema> {
    map_object(schema, |object| object.max_length = Some(value))
}

pub fn with_pattern(schema: RefOr<Schema>, value: &str) -> RefOr<Schema> {
    map_object(schema, |object| object.pattern = Some(value.to_string()))
}

pub fn with_minimum(schema: RefOr<Schema>, value: f64) -> RefOr<Schema> {
    map_object(schema, |object| object.minimum = Some(value.into()))
}

pub fn with_maximum(schema: RefOr<Schema>, value: f64) -> RefOr<Schema> {
    map_object(schema, |object| object.maximum = Some(value.into()))
}

pub fn with_exclusive_minimum(schema: RefOr<Schema>, value: f64) -> RefOr<Schema> {
    map_object(schema, |object| {
        object.exclusive_minimum = Some(value.into())
    })
}

pub fn with_exclusive_maximum(schema: RefOr<Schema>, value: f64) -> RefOr<Schema> {
    map_object(schema, |object| {
        object.exclusive_maximum = Some(value.into())
    })
}

pub fn with_min_items(schema: RefOr<Schema>, value: usize) -> RefOr<Schema> {
    map_array(schema, |array| array.min_items = Some(value))
}

pub fn with_max_items(schema: RefOr<Schema>, value: usize) -> RefOr<Schema> {
    map_array(schema, |array| array.max_items = Some(value))
}

pub fn with_unique_items(schema: RefOr<Schema>) -> RefOr<Schema> {
    map_array(schema, |array| array.unique_items = true)
}

pub fn with_format(schema: RefOr<Schema>, format: &str) -> RefOr<Schema> {
    let format = match format {
        "email" => SchemaFormat::KnownFormat(KnownFormat::Email),
        "uuid" => SchemaFormat::Custom("uuid".to_string()),
        "uri" => SchemaFormat::Custom("uri".to_string()),
        "url" => SchemaFormat::Custom("uri".to_string()),
        "date" => SchemaFormat::KnownFormat(KnownFormat::Date),
        "date-time" => SchemaFormat::KnownFormat(KnownFormat::DateTime),
        "ipv4" => SchemaFormat::KnownFormat(KnownFormat::Ipv4),
        "ipv6" => SchemaFormat::KnownFormat(KnownFormat::Ipv6),
        "byte" => SchemaFormat::KnownFormat(KnownFormat::Byte),
        "binary" => SchemaFormat::KnownFormat(KnownFormat::Binary),
        other => SchemaFormat::Custom(other.to_string()),
    };
    map_object(schema, |object| object.format = Some(format))
}

pub fn with_extension(schema: RefOr<Schema>, key: &str, value: Value) -> RefOr<Schema> {
    fn extension_map(
        existing: Option<utoipa::openapi::extensions::Extensions>,
        key: &str,
        value: Value,
    ) -> utoipa::openapi::extensions::Extensions {
        let mut map: std::collections::HashMap<String, Value> = existing
            .map(|extensions| {
                extensions
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        map.insert(key.to_string(), value);
        serde_json::from_value(Value::Object(map.into_iter().collect())).unwrap_or_default()
    }

    match schema {
        RefOr::T(Schema::Object(mut object)) => {
            object.extensions = Some(extension_map(object.extensions.take(), key, value));
            RefOr::T(Schema::Object(object))
        }
        RefOr::T(Schema::Array(mut array)) => {
            array.extensions = Some(extension_map(array.extensions.take(), key, value));
            RefOr::T(Schema::Array(array))
        }
        other => other,
    }
}

/// Apply a roundhouse-only constraint (`unique_items`, `x-validate-with`) to a property of an
/// already-built utoipa schema.
///
/// Schema shape comes from `utoipa::ToSchema`; these keywords are not part of utoipa's derive
/// attribute set, so they are patched in after the component is built. The search descends into
/// `oneOf`/`allOf` so a named field in an enum variant is still reachable.
pub fn patch_property(
    schema: &mut RefOr<Schema>,
    name: &str,
    update: &mut dyn FnMut(RefOr<Schema>) -> RefOr<Schema>,
) {
    match schema {
        RefOr::T(Schema::Object(object)) => {
            if let Some(property) = object.properties.get_mut(name) {
                let current = property.clone();
                *property = update(current);
            }
        }
        RefOr::T(Schema::OneOf(one_of)) => {
            for item in &mut one_of.items {
                patch_property(item, name, update);
            }
        }
        RefOr::T(Schema::AllOf(all_of)) => {
            for item in &mut all_of.items {
                patch_property(item, name, update);
            }
        }
        _ => {}
    }
}

pub fn with_example(schema: RefOr<Schema>, example: Value) -> RefOr<Schema> {
    match schema {
        RefOr::T(Schema::Object(mut object)) => {
            object.example = Some(example);
            RefOr::T(Schema::Object(object))
        }
        RefOr::T(Schema::Array(mut array)) => {
            array.example = Some(example);
            RefOr::T(Schema::Array(array))
        }
        other => other,
    }
}

/// A string schema whose only allowed value is `value` (a discriminator tag constant).
pub fn string_enum(value: &str) -> RefOr<Schema> {
    ObjectBuilder::new()
        .schema_type(Type::String)
        .enum_values(Some([serde_json::json!(value)]))
        .into()
}

/// Build an object schema from pre-built properties.
pub fn object_with(
    properties: Vec<(String, RefOr<Schema>)>,
    required: Vec<String>,
) -> RefOr<Schema> {
    let mut builder = object();
    for (name, schema) in properties {
        builder = builder.property(name, schema);
    }
    for name in required {
        builder = builder.required(name);
    }
    builder.into()
}

/// Merge extra properties into a schema. Objects are updated in place; anything else becomes an
/// `allOf` with a small object carrying the extra properties.
pub fn with_properties(
    schema: RefOr<Schema>,
    properties: Vec<(String, RefOr<Schema>)>,
    required: Vec<String>,
) -> RefOr<Schema> {
    let extras = object_with(properties, required);
    match schema {
        RefOr::T(Schema::Object(mut object)) => {
            if let RefOr::T(Schema::Object(extra)) = extras {
                object.properties.extend(extra.properties);
                object.required.extend(extra.required);
            }
            RefOr::T(Schema::Object(object))
        }
        other => {
            let mut all = utoipa::openapi::schema::AllOfBuilder::new();
            all = all.item(other);
            all = all.item(extras);
            all.into()
        }
    }
}
