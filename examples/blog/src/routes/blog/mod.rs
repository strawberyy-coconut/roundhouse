use roundhouse::prelude::*;

pub mod admin;
pub mod authors;
pub mod metrics;
pub mod posts;

mount!("/api/blog", tags = ["blog"]);
