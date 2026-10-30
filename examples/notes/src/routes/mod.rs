use roundhouse::prelude::*;

pub mod health;
pub mod notes;

// The root module composes the subtree explicitly. It owns no URL segment, only shared policy.
routes! {
    tags = ["api"],
    ...health,
    ...notes,
}
