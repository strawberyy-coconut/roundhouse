use roundhouse::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, PathParams)]
pub struct Wrong {
    pub other: u64,
}

routes! {
    prefix = "/ui",
 get("/{id}", show) }

#[handler]
pub async fn show(Path(Wrong { other }): Path<Wrong>) -> String {
    other.to_string()
}

fn main() {}
