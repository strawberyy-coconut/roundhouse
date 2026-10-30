use roundhouse::prelude::*;

pub mod profiles;

routes! {
    tags = ["examples"],
    ...profiles,
}
