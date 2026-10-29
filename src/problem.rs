//! RFC 9457 `application/problem+json` responses.

pub use roundhouse_problem::{Problem, problem_schema, problem_value};

impl crate::doc::ApiResponse for Problem {
    /// `Problem` is the framework's default error type, so it contributes a `default` response
    /// with the `application/problem+json` schema. A `Result<_, Problem>` therefore documents the
    /// failures it can produce instead of only its success status.
    fn responses(
        components: &mut utoipa::openapi::schema::Components,
    ) -> Vec<crate::doc::ResponseDoc> {
        let _ = components;
        vec![crate::doc::ResponseDoc::problem_default()]
    }
}

// Join the same link-time registry the derives use, so `stored_responses_for::<Problem>` finds
// this implementation.
inventory::submit! {
    crate::registry::ResponseRecord::__new(
        std::any::type_name::<Problem>,
        <Problem as crate::doc::ApiResponse>::responses,
    )
}
