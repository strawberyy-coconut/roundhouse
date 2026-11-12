//! The concrete-type response probe used by `#[handler]`.
//!
//! `ApiResponse` is the public contract; this probe is the autoref fallback that keeps a
//! hand-written `IntoResponse` return type compiling and marks it undocumented. It is macro
//! machinery, not an extension point.

use crate::doc::{ApiResponse, ResponseDoc, SchemaContext};

pub struct ResponseProbe<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> ResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for ResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeResponses {
    /// `Some` when the type implements [`ApiResponse`], including a known type that documents no
    /// response (for example `Option<T>` or `Infallible`). `None` means the document cannot see
    /// the response type at all.
    fn probe_responses(self, components: &mut dyn SchemaContext) -> Option<Vec<ResponseDoc>>;
}

impl<T: ApiResponse + ?Sized> ProbeResponses for &ResponseProbe<T> {
    fn probe_responses(self, components: &mut dyn SchemaContext) -> Option<Vec<ResponseDoc>> {
        Some(T::responses(components))
    }
}

impl<T: ?Sized> ProbeResponses for &&ResponseProbe<T> {
    fn probe_responses(self, _components: &mut dyn SchemaContext) -> Option<Vec<ResponseDoc>> {
        None
    }
}
