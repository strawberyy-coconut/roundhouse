use roundhouse::prelude::*;

#[derive(ApiResponse)]
enum Response {
    #[status(999)]
    Invalid,
}

fn main() {}
