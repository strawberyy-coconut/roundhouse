use roundhouse::prelude::*;

routes! {
    prefix = "/x",

    "/a" => { get(first, operation_id = "same") },
    "/b" => { get(second, operation_id = "same") },
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
