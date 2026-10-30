use roundhouse::prelude::*;

pub mod entry;
pub mod full;
pub mod route_layer;

routes! {
    ...entry,
    ...full,
    ...route_layer,
}
