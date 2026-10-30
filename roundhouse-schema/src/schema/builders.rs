use serde_json::Value;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{
    AdditionalProperties, ArrayBuilder, KnownFormat, Object, ObjectBuilder, OneOfBuilder, Ref,
    Schema, SchemaFormat, SchemaType, Type,
};

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
