use roundhouse::prelude::*;

pub mod blog;
pub mod health;
pub mod legacy;

// A group mount: crate-wide tags, no URL segment.
mount!(tags = ["api"]);
