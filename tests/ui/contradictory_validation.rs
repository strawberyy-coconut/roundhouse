use roundhouse::prelude::*;

roundhouse::app_state!(());

#[handler]
async fn x(Json(_body): Json<String>) -> &'static str {
    "x"
}

routes! {
    "/x" => {
        post(x, validate = false, validate_body = true),
    },
}

fn main() {}
