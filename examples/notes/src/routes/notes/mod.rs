use roundhouse::prelude::*;

pub mod create;
pub mod list;
pub mod show;

// The module owns its prefix and tags; every leaf is composed explicitly below it.
routes! {
    prefix = "/notes",
    tags = ["notes"],
    ...create,
    ...list,
    ...show,
}
