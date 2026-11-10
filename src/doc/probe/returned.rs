//! The return-type probe.
//!
//! The handler macro can classify a literal `Result<T, E>` syntactically, but a type alias hides
//! the name. Traits see through aliases, so this probe documents `Result`-shaped returns whose
//! two sides both implement [`ApiResponse`](crate::doc::ApiResponse), and falls back to the
//! ordinary `ApiResponse` probe for every other type.

use utoipa::openapi::schema::Components;

use crate::doc::{ApiResponse, HandlerDocs, ProbeResponses, ResponseProbe};

pub struct ReturnProbe<T>(std::marker::PhantomData<T>);

impl<T> ReturnProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ReturnProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeReturn {
    /// `true` when the type described itself, even if it documented no responses.
    fn probe_return(self, components: &mut Components, docs: &mut HandlerDocs) -> bool;
}

/// A result-shaped return type. Both sides must implement `ApiResponse`; otherwise the fallback
/// returns `false` and the operation is checked for a declared success response.
impl<T: ApiResponse, E: ApiResponse> ProbeReturn for &ReturnProbe<Result<T, E>> {
    fn probe_return(self, components: &mut Components, docs: &mut HandlerDocs) -> bool {
        if let Some(responses) = (&ResponseProbe::<T>::new()).probe_responses(components) {
            docs.responses.extend(responses);
        }
        if let Some(responses) = (&ResponseProbe::<E>::new()).probe_responses(components) {
            docs.responses.extend(responses);
        } else {
            docs.undocumented_error = Some(std::any::type_name::<E>());
        }
        true
    }
}

impl<T: ApiResponse> ProbeReturn for &&ReturnProbe<T> {
    fn probe_return(self, components: &mut Components, docs: &mut HandlerDocs) -> bool {
        if let Some(responses) = (&ResponseProbe::<T>::new()).probe_responses(components) {
            docs.responses.extend(responses);
        }
        true
    }
}

impl<T> ProbeReturn for &&&ReturnProbe<T> {
    fn probe_return(self, _components: &mut Components, _docs: &mut HandlerDocs) -> bool {
        false
    }
}
