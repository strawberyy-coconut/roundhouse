use roundhouse::prelude::*;

#[derive(QueryParams)]
struct Filter {
    #[schema(min_length = -1)]
    name: String,
}

fn main() {}
