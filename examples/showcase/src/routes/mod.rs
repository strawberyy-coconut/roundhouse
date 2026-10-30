use roundhouse::prelude::*;

pub mod collisions;
pub mod events;
pub mod hidden;
pub mod misc;

routes! {
    tags = ["showcase"],
    ...collisions::a,
    ...collisions::b,
    ...events,
    ...hidden,
    ...misc,
}
