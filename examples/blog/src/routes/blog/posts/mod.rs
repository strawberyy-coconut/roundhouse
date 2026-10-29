use roundhouse::prelude::*;

pub mod item;
pub mod list;

mount!("/posts", tags = ["posts"]);
