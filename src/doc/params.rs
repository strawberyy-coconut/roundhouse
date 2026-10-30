use utoipa::openapi::schema::Components;

use super::{Param, PathNames};

pub fn path_params<T: PathNames>() -> Vec<Param> {
    T::params()
}

/// Query parameters declared by `#[derive(QueryParams)]`.
pub trait QueryParams {
    fn params(components: &mut Components) -> Vec<Param>;

    /// Run the custom `validate_with` hooks; declarative constraints come from the parameter
    /// schema.
    fn validate_at(&self, _pointer: &str, _errors: &mut crate::validate::ValidationErrors) {}

    fn validate(&self) -> Result<(), crate::validate::ValidationErrors> {
        let mut errors = crate::validate::ValidationErrors::new();
        self.validate_at("", &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl<T: QueryParams> QueryParams for Option<T> {
    fn params(components: &mut Components) -> Vec<Param> {
        let mut params = T::params(components);
        for param in &mut params {
            param.required = false;
        }
        params
    }

    fn validate_at(&self, pointer: &str, errors: &mut crate::validate::ValidationErrors) {
        if let Some(value) = self {
            value.validate_at(pointer, errors);
        }
    }
}

pub fn stored_query_params_for<T: QueryParams + ?Sized>(components: &mut Components) -> Vec<Param> {
    <T as QueryParams>::params(components)
}
