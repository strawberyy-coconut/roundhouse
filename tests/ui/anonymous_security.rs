use roundhouse::prelude::*;

roundhouse::app_state!(());

struct Bearer;

#[handler]
async fn x() -> &'static str {
    "x"
}

routes! {
    anonymous,
    security = [Bearer],

    "/x" => {
        get(x),
    },
}

fn main() {}
