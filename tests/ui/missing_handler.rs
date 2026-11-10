use roundhouse::prelude::*;

routes! {
    prefix = "/ui",
 "/x" => { get(show) } }

pub async fn show() -> &'static str {
    "x"
}

fn main() {}
