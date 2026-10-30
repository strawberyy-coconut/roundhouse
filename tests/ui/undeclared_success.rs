use roundhouse::prelude::*;

routes! {
    prefix = "/undeclared",
 get("/x", x, operation_id = "undeclared_x") }

#[handler]
async fn x() -> impl IntoResponse {
    "x"
}

fn main() {}
