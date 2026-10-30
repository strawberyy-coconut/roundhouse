//! RFC 9457 `application/problem+json` responses.
//!
//! [`Problem`] is the one error body the framework produces: malformed JSON, a wrong
//! `Content-Type`, a missing query parameter and a failed constraint all arrive in this shape, so
//! a client needs exactly one parser. A `Result<_, Problem>` therefore documents a `default`
//! response instead of only its success status.

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
