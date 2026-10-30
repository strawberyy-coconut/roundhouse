use roundhouse::prelude::*;

pub mod admin;
pub mod authors;
pub mod metrics;
pub mod posts;

routes! {
    prefix = "/api/blog",
    tags = ["blog"],
    ...admin,
    ...authors,
    ...metrics,
    ...posts,
}
