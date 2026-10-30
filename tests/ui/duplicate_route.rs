use roundhouse::prelude::*;

mount!("/x");

routes! {
    get("/a", first),
    get("/a", second),
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
