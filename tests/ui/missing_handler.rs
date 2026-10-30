use roundhouse::prelude::*;

routes! {
    prefix = "/ui",
 get("/x", show) }

pub async fn show() -> &'static str {
    "x"
}

fn main() {}
