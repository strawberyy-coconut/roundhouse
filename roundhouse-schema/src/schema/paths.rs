use std::collections::HashMap;

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

use crate::validate::ValidationErrors;

use super::{ApiSchema, PathNames, PathValidate};

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

    fn params() -> Vec<crate::Param> {
        let mut params = T::params();
        for param in &mut params {
            param.required = false;
        }
        params
    }
}
