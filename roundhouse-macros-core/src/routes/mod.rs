//! Parsing and expansion for `routes!`.

mod expand;
mod parse;
mod validate;

pub use expand::expand;
pub use parse::RoutesInput;
