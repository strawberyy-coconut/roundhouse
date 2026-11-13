use roundhouse::prelude::*;

#[derive(ApiResponse)]
enum Bad {
    #[status(200)]
    Both {
        #[body]
        first: String,
        #[body]
        second: String,
    },
}

fn main() {}
