pub mod routes;
pub mod schemes;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use roundhouse::openapi::Info;
use schemes::User;

#[derive(Clone)]
pub struct AppState {
    tokens: Arc<HashMap<String, User>>,
    api_keys: Arc<HashSet<String>>,
}

impl Default for AppState {
    fn default() -> Self {
        let tokens = HashMap::from([
            (
                "reader".to_string(),
                User {
                    name: "reader".to_string(),
                    admin: false,
                },
            ),
            (
                "admin".to_string(),
                User {
                    name: "admin".to_string(),
                    admin: true,
                },
            ),
        ]);
        Self {
            tokens: Arc::new(tokens),
            api_keys: Arc::new(HashSet::from(["secret".to_string()])),
        }
    }
}

impl AppState {
    pub fn user(&self, token: &str) -> Option<User> {
        self.tokens.get(token).cloned()
    }

    pub fn api_key(&self, key: &str) -> bool {
        self.api_keys.contains(key)
    }
}

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state)
        .openapi(|| Info::new("Auth API", env!("CARGO_PKG_VERSION")))
        .docs()
}
