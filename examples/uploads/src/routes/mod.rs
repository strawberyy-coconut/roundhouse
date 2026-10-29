use roundhouse::prelude::*;

pub mod uploads;

mount!(
    "/uploads",
    tags = ["uploads"],
    doc = ::roundhouse::doc::layer::body_limit(413),
);
