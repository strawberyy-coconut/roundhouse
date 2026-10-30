use roundhouse::prelude::*;

pub mod guarded;
pub mod protected;

routes! {
    tags = ["auth"],
    ...guarded,
    ...protected,
}
