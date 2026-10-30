use roundhouse::prelude::*;

routes! {
    prefix = "/x",

    get("/items/{id}", first),
    get("/items/{post_id}", second),
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
