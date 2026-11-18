//! The runtime parameter contract.
//!
//! [`Params`] is what `Path<T>`, `Query<T>` and `RepeatedQuery<T>` require at runtime: the
//! capture names and the value that the contract-backed validation engine sees. Schema
//! descriptions live in the build-only [`schema::ParamsDoc`](crate::schema::ParamsDoc)
//! trait, so a production build does not compile any schema derivation.

use crate::validate::ValidationErrors;

/// A value that can carry custom validation hooks.
///
/// The derived schema types implement this beside their documentation impl, so `validate_with`
/// hooks still run in a production build where schema derivation is compiled out.
pub trait ValidateHook {
    /// Run the type's hooks against `value`, writing into `errors` with a JSON Pointer base.
    fn validate_at(&self, _pointer: &str, _errors: &mut ValidationErrors) {}

    /// Run the type's hooks and collect every violation.
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

/// A path, query or form type: its capture names and its wire value.
///
/// The runtime engine only needs these two facts. Documentation and JSON Schema derivation are
/// supplied by the build-only `ParamsDoc` trait.
pub trait Params {
    /// Capture names declared by the type, in template order.
    const NAMES: &'static [&'static str] = &[];

    /// The value the contract validation engine validates.
    fn parts_value(&self) -> serde_json::Value;
}

macro_rules! scalar_param {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ValidateHook for $ty {}

            impl Params for $ty {
                fn parts_value(&self) -> serde_json::Value {
                    serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
                }
            }
        )*
    };
}

scalar_param!(
    bool, char, String, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

#[cfg(feature = "uuid")]
scalar_param!(uuid::Uuid);

#[cfg(feature = "chrono")]
scalar_param!(
    chrono::NaiveDate,
    chrono::NaiveDateTime,
    chrono::DateTime<chrono::Utc>,
);

#[cfg(feature = "time")]
scalar_param!(time::Date, time::OffsetDateTime, time::PrimitiveDateTime,);

#[cfg(feature = "jiff")]
scalar_param!(
    jiff::Timestamp,
    jiff::Zoned,
    jiff::civil::Date,
    jiff::civil::DateTime,
    jiff::civil::Time,
);

#[cfg(feature = "decimal")]
scalar_param!(rust_decimal::Decimal);

impl ValidateHook for serde_json::Value {}

impl Params for serde_json::Value {
    fn parts_value(&self) -> serde_json::Value {
        self.clone()
    }
}

impl<K, V> ValidateHook for std::collections::HashMap<K, V> {}

impl<K, V> Params for std::collections::HashMap<K, V>
where
    K: serde::Serialize + Eq + std::hash::Hash,
    V: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl<K, V> ValidateHook for std::collections::BTreeMap<K, V> {}

impl<K, V> Params for std::collections::BTreeMap<K, V>
where
    K: serde::Serialize + Ord,
    V: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl<T> ValidateHook for Vec<T> {}

impl<T> Params for Vec<T>
where
    T: serde::Serialize,
{
    fn parts_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

impl<T: Params> Params for Option<T> {
    const NAMES: &'static [&'static str] = T::NAMES;

    fn parts_value(&self) -> serde_json::Value {
        match self {
            Some(value) => value.parts_value(),
            None => serde_json::Value::Null,
        }
    }
}

impl<T: ValidateHook> ValidateHook for Option<T> {
    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {
        if let Some(value) = self {
            value.validate_at(pointer, errors);
        }
    }
}
