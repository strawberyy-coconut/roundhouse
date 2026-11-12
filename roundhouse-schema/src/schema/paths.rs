use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

use super::{ApiSchema, Params, SchemaContext};

/// `Params` for scalar path params.
///
/// The type carries its schema but not a name: `routes!` fills the template's capture names in,
/// so `Path<u64>` produces a documented integer parameter instead of an invalid document.
macro_rules! scalar_params {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Params for $ty {
                fn scalar_schema() -> Option<RefOr<Schema>> {
                    Some(<$ty as ApiSchema>::schema())
                }

                fn parts_value(&self) -> serde_json::Value {
                    serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
                }
            }
        )*
    };
}

scalar_params!(
    bool, char, String, i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

#[cfg(feature = "uuid")]
scalar_params!(uuid::Uuid);

#[cfg(feature = "chrono")]
scalar_params!(
    chrono::NaiveDate,
    chrono::NaiveDateTime,
    chrono::DateTime<chrono::Utc>,
);

#[cfg(feature = "time")]
scalar_params!(time::Date, time::OffsetDateTime, time::PrimitiveDateTime,);

#[cfg(feature = "jiff")]
scalar_params!(
    jiff::Timestamp,
    jiff::Zoned,
    jiff::civil::Date,
    jiff::civil::DateTime,
    jiff::civil::Time,
);

#[cfg(feature = "decimal")]
scalar_params!(rust_decimal::Decimal);

impl<T: Params> Params for Option<T> {
    const NAMES: &'static [&'static str] = T::NAMES;

    fn register(context: &mut dyn SchemaContext) {
        T::register(context);
    }

    fn params(context: &mut dyn SchemaContext) -> Vec<crate::Param> {
        let mut params = T::params(context);
        for param in &mut params {
            param.required = false;
        }
        params
    }

    fn scalar_schema() -> Option<RefOr<Schema>> {
        T::scalar_schema()
    }

    fn parts_schema(context: &mut dyn SchemaContext) -> RefOr<Schema> {
        T::parts_schema(context)
    }

    fn parts_value(&self) -> serde_json::Value {
        match self {
            Some(value) => value.parts_value(),
            None => serde_json::Value::Null,
        }
    }

    fn validate_at(&self, pointer: &str, errors: &mut crate::validate::ValidationErrors) {
        if let Some(value) = self {
            value.validate_at(pointer, errors);
        }
    }

    fn parts_doc() -> Option<crate::schema::PartsDoc> {
        T::parts_doc()
    }
}
