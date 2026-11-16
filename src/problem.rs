//! RFC 9457 `application/problem+json` responses.
//!
//! [`Problem`] is the one error body the framework produces: malformed JSON, a wrong
//! `Content-Type`, a missing query parameter and a failed constraint all arrive in this shape, so
//! a client needs exactly one parser. A `Result<_, Problem>` therefore documents a `default`
//! response instead of only its success status.

pub use roundhouse_problem::Problem;
pub use roundhouse_problem::problem_value;
#[cfg(feature = "contract")]
pub use roundhouse_problem::{problem_schema, problem_schema_ref};

#[cfg(feature = "contract")]
impl crate::doc::ResponseDocSource for Problem {
    const SUCCESS: &'static [u16] = &[];

    /// `Problem` is the framework's default error type, so it contributes a `default` response
    /// with the `application/problem+json` schema. A `Result<_, Problem>` therefore documents the
    /// failures it can produce instead of only its success status.
    fn responses(_context: &mut dyn crate::doc::SchemaContext) -> Vec<crate::doc::ResponseDoc> {
        vec![crate::doc::ResponseDoc::problem_default()]
    }
}
