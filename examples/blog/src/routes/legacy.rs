use roundhouse::prelude::*;

routes! {
    get("/legacy", legacy, absolute, tags = ["legacy"]),
}

/// `absolute` ignores every ancestor mount, including `/api/blog`.
#[handler]
pub async fn legacy() -> &'static str {
    "legacy route"
}
