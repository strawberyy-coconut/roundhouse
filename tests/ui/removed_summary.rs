use roundhouse::prelude::*;

roundhouse::app_state!(());

#[handler]
async fn x() -> &'static str {
    "x"
}

routes! {
    "/x" => {
        get(x, summary = "x"),
    },
}

fn main() {}
