use roundhouse::prelude::*;

/// @summary: One.
/// @summary: Two.
#[handler]
async fn x() -> &'static str {
    "x"
}

routes! {
    "/x" => {
        get(x),
    },
}

fn main() {}
