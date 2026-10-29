use roundhouse::prelude::*;

mount!("/ui");
routes! { get("/x", show) }

pub async fn show() -> &'static str {
    "x"
}

fn main() {}
