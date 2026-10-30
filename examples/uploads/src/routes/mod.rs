use roundhouse::prelude::*;

pub mod uploads;

routes! {
    prefix = "/uploads",
    tags = ["uploads"],
    doc = ::roundhouse::doc::layer::body_limit(413),
    ...uploads,
}
