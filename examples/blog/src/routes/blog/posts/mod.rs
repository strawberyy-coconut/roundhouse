use roundhouse::prelude::*;

pub mod item;
pub mod list;

routes! {
    prefix = "/posts",
    tags = ["posts"],
    ...item,
    ...list,
}
