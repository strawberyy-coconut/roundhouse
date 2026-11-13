use roundhouse::prelude::*;

roundhouse::app_state!(());

struct Note;

#[handler(response(200, Note), response(200, Note))]
async fn show() -> &'static str {
    "x"
}

fn main() {}
