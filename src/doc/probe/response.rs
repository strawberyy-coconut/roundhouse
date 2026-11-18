//! The concrete-type response probe used by `#[handler]`.
//!
//! `DocumentedResponse` is the public contract; this probe is the autoref fallback that keeps a
//! hand-written `IntoResponse` return type compiling and marks it undocumented. It is macro
//! machinery, not an extension point.

use crate::doc::{DocumentedError, DocumentedResponse, ResponseDoc, SchemaContext};

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
    /// `Some` when the type implements [`DocumentedResponse`], including a known type that documents no
    /// response (for example `Option<T>` or `Infallible`). `None` means the document cannot see
    /// the response type at all.
    fn probe_responses(self, components: &mut dyn SchemaContext) -> Option<Vec<ResponseDoc>>;
}

impl<T: DocumentedResponse + ?Sized> ProbeResponses for &ResponseProbe<T> {
    fn probe_responses(self, components: &mut dyn SchemaContext) -> Option<Vec<ResponseDoc>> {
        Some(T::responses(components))
    }
}

impl<T: ?Sized> ProbeResponses for &&ResponseProbe<T> {
    fn probe_responses(self, _components: &mut dyn SchemaContext) -> Option<Vec<ResponseDoc>> {
        None
    }
}

/// The declared-response probe used by `#[handler(response(status, Type))]`.
///
/// A declaration may name a success type or an error type. The success impl is the first autoref
/// candidate, so a type that implements both keeps the previous `DocumentedResponse` behaviour;
/// an error-only type falls through to the `DocumentedError` impl. A type implementing neither
/// fails to compile with the trait's diagnostic.
pub struct DeclaredResponseProbe<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> DeclaredResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for DeclaredResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeDeclaredResponses {
    fn probe_declared_responses(self, components: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}

impl<T: DocumentedResponse + ?Sized> ProbeDeclaredResponses for &DeclaredResponseProbe<T> {
    fn probe_declared_responses(self, components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        T::responses(components)
    }
}

impl<T: DocumentedError + ?Sized> ProbeDeclaredResponses for &&DeclaredResponseProbe<T> {
    fn probe_declared_responses(self, components: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        T::responses(components)
    }
}
