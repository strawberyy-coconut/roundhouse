use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Address {
    #[schema(min_length = 2, max_length = 80)]
    pub street: String,
    #[schema(pattern = r"^[0-9]{5}$")]
    pub postal_code: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Reader,
    Author,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    #[schema(
        min_length = 3,
        max_length = 32,
        pattern = r"^[a-z0-9_-]+$",
        example = "ada_lovelace"
    )]
    pub username: String,
    #[schema(format = "email")]
    pub email: String,
    #[schema(format = "url")]
    pub website: Option<String>,
    #[schema(format = "uuid")]
    pub id: String,
    #[schema(minimum = 18, maximum = 130)]
    pub age: u8,
    #[schema(exclusive_minimum = 0, exclusive_maximum = 10)]
    pub score: f64,
    #[schema(min_items = 1, max_items = 5, unique_items)]
    pub tags: Vec<String>,
    pub address: Address,
    pub nickname: Option<String>,
    pub role: Role,
}
