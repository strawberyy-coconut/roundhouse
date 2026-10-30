use roundhouse::prelude::*;

pub mod blog;
pub mod health;
pub mod legacy;

// The root module composes every subtree. It owns no URL segment, only crate-wide tags.
routes! {
    tags = ["api"],
    ...blog,
    ...health,
    ...legacy,
}
