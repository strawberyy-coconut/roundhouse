//! Component-name collision records.
//!
//! The ledger itself lives in [`DocContext`](crate::doc::DocContext); this module keeps the
//! public record type and re-exports [`Claim`] for derive expansions.

use crate::build_error::Source;

pub use roundhouse_schema::Claim;

/// A component name claimed by two different Rust types.
#[derive(Clone, Debug)]
pub struct Collision {
    pub name: String,
    pub first_type: &'static str,
    pub second_type: &'static str,
    pub source: Source,
}
