use roundhouse::prelude::*;

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
