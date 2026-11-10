//! The concrete-type bridge between `#[handler]` and [`crate::doc::ExtractArg`].
//!
//! The macro sees only the token `T` of an argument. This probe resolves at the concrete type:
//! when `T` implements [`ExtractArg`], its documentation and validation run; otherwise the
//! argument is marked as an extractor the document cannot infer, exactly like the old
//! `ProbeExtractor` fallback. Keeping the fallback here means a custom or foreign extractor still
//! compiles, and `strict()` can turn the marker into a build error.

use utoipa::openapi::schema::Components;

use crate::doc::{ExtractArg, HandlerDocs};
use crate::extract::ValidationContext;
use crate::problem::Problem;

pub struct ArgProbe<T>(std::marker::PhantomData<T>);

impl<T> ArgProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ArgProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeArg<T> {
    fn document(self, components: &mut Components, docs: &mut HandlerDocs);
    #[allow(clippy::result_large_err)]
    fn validate(self, value: &T, context: &ValidationContext) -> Result<(), Problem>;
    fn body_arg(self) -> bool;
    fn verified(self) -> bool;
}

impl<T: ExtractArg> ProbeArg<T> for &ArgProbe<T> {
    fn document(self, components: &mut Components, docs: &mut HandlerDocs) {
        T::document(components, docs);
    }

    fn validate(self, value: &T, context: &ValidationContext) -> Result<(), Problem> {
        value.validate(context)
    }

    fn body_arg(self) -> bool {
        T::IS_BODY
    }

    fn verified(self) -> bool {
        T::SHIM_VALIDATES
    }
}

impl<T> ProbeArg<T> for &&ArgProbe<T> {
    fn document(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.unverified_extractor = true;
    }

    fn validate(self, _value: &T, _context: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }

    fn body_arg(self) -> bool {
        false
    }

    fn verified(self) -> bool {
        false
    }
}
