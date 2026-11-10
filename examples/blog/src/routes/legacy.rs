use roundhouse::prelude::*;

routes! {
    "/legacy" => { get(legacy, absolute, tags = ["legacy"]) },
}

/// `absolute` ignores every ancestor module prefix, including `/api/blog`.
#[handler]
pub async fn legacy() -> &'static str {
    "legacy route"
}
