use roundhouse::prelude::*;

pub mod health;
pub mod notes;

// A group mount: policy for the whole subtree, no URL segment of its own.
mount!(tags = ["api"]);
