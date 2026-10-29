use roundhouse::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, PathParams)]
pub struct Bad {
    #[serde(default)]
    pub id: u64,
}

fn main() {}
