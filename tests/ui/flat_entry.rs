use roundhouse::prelude::*;

roundhouse::app_state!(());

#[handler]
async fn x() -> &'static str {
    "x"
}

routes! {
    get("/x", x),
}

fn main() {}
