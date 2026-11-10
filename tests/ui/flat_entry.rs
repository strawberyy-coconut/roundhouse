use roundhouse::prelude::*;

#[handler]
async fn x() -> &'static str {
    "x"
}

routes! {
    get("/x", x),
}

fn main() {}
