use roundhouse::prelude::*;

roundhouse::app_state!(());

struct Note;

/// @responses:
///   200: Note
#[handler(response(200, Note))]
async fn show() -> &'static str {
    "x"
}

fn main() {}
