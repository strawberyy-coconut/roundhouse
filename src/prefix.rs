//! Path joining, template validation and duplicate-shape detection.
//!
//! The implementation lives in `roundhouse-path` so the macro's compile-time checks and the
//! runtime resolver share one definition. This module is the facade's stable path.

pub use roundhouse_path::{
    assert_path_names, captures, has_catch_all, join, normalise_template, path_names_match,
    validate_prefix, validate_template,
};
