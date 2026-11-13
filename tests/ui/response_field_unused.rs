use roundhouse::prelude::*;

#[derive(ApiResponse)]
enum Bad {
    #[status(404)]
    NotFound { id: i64 },
}

fn main() {}
