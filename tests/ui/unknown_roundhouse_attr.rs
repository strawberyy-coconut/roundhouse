use roundhouse::prelude::*;

roundhouse::app_state!(());

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Bad {
    #[roundhouse(min_lenght = 3)]
    pub name: String,
}

fn main() {}
