use roundhouse::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, ApiSchema)]
pub struct Bad {
    #[schema(skip_validation)]
    pub title: String,
}

fn main() {}
