use roundhouse::interop::ToSchema;
#[roundhouse::schema]
#[derive(serde::Serialize, roundhouse::ToSchema)]
pub struct Bad {
    pub name: String,
}

fn main() {}
