//! Reusable response wrappers for the common created/no-content cases.
//!
//! The named wrappers are generated in [`crate::status`] from the vendored IANA registry and
//! re-exported here for compatibility with earlier roundhouse versions.
//!
//! [`Created`] is a `201` JSON response, [`NoContent`] is a bodyless `204`, and
//! [`Status<CODE, T>`](Status) / [`Empty<CODE>`](Empty) cover every other status code.

pub use crate::status::{Created, Empty, NoContent, Status};
