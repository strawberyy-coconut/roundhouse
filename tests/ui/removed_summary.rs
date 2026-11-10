use roundhouse::prelude::*;

#[handler]
async fn x() -> &'static str {
    "x"
}

routes! {
    "/x" => {
        get(x, summary = "x"),
    },
}

fn main() {}
