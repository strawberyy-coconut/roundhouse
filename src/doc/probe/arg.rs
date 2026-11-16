//! The concrete-type bridge between `#[handler]` and [`crate::doc::ExtractDoc`].
//!
//! The macro sees only the token `T` of an argument. This probe resolves at the concrete type:
//! when `T` implements `ExtractDoc`, its documentation runs; otherwise the argument is marked as
//! an extractor the document cannot infer. Keeping the fallback here means a custom or foreign
//! extractor still compiles, and `strict()` can turn the marker into a build error.

use crate::doc::{ArgDoc, DocContext, ExtractDoc};

pub struct ArgDocProbe<T>(std::marker::PhantomData<T>);

impl<T> ArgDocProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ArgDocProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeArgDoc<T> {
    fn document(self, components: &mut DocContext<'_>) -> ArgDoc;
}

impl<T: ExtractDoc> ProbeArgDoc<T> for &ArgDocProbe<T> {
    fn document(self, components: &mut DocContext<'_>) -> ArgDoc {
        T::document(components)
    }
}

impl<T> ProbeArgDoc<T> for &&ArgDocProbe<T> {
    fn document(self, _components: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            unverified_extractor: true,
            ..ArgDoc::default()
        }
    }
}
