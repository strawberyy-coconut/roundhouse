use roundhouse::prelude::*;

mount!("/undeclared");

routes! { get("/x", x, operation_id = "undeclared_x") }

#[handler]
async fn x() -> impl IntoResponse {
    "x"
}

fn main() {}
