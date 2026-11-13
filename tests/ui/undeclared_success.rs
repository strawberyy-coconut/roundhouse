use roundhouse::prelude::*;

roundhouse::app_state!(());

routes! {
    prefix = "/undeclared",
 "/x" => { get(x, operation_id = "undeclared_x") } }

#[handler]
async fn x() -> impl IntoResponse {
    "x"
}

fn main() {}
