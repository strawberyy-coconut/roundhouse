use roundhouse::prelude::*;

mount!("/x");

routes! {
    get("/a", first, operation_id = "same"),
    get("/b", second, operation_id = "same"),
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
