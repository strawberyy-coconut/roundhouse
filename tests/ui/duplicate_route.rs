use roundhouse::prelude::*;

routes! {
    prefix = "/x",

    "/a" => { get(first) },
    "/a" => { get(second) },
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
