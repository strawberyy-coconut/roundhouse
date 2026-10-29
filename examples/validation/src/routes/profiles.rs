use roundhouse::prelude::*;

use crate::domain::Profile;
use crate::error::ApiError;

mount!("/profiles", tags = ["profiles"]);

routes! {
    post("/", create, summary = "Create a profile"),
    post(
        "/unchecked",
        unchecked,
        summary = "Create a profile without constraint checks",
        validate = false,
    ),
}

#[handler]
pub async fn create(Json(profile): Json<Profile>) -> Result<Json<Profile>, ApiError> {
    if profile.username == "root" {
        return Err(ApiError::UsernameReserved {
            username: profile.username,
        });
    }
    Ok(Json(profile))
}

/// The same extractor, with `validate = false` on the table entry. This still deserializes and
/// still rejects malformed JSON; it only skips the `#[schema(..)]` constraint checks.
#[handler]
pub async fn unchecked(Json(profile): Json<Profile>) -> Json<Profile> {
    Json(profile)
}
