use roundhouse::prelude::*;

roundhouse::app_state!(());

routes! {
    "/api" [tags = ["api"]] {
        "/x" => { get(x) },
    }
}

#[handler]
async fn x() -> &'static str {
    "x"
}

fn main() {}
