use roundhouse::prelude::*;

roundhouse::app_state!(());

routes! {
    prefix = "/x",

    "/items/{id}" => { get(first) },
    "/items/{post_id}" => { get(second) },
}

#[handler]
async fn first() -> &'static str {
    "a"
}

#[handler]
async fn second() -> &'static str {
    "b"
}

fn main() {}
