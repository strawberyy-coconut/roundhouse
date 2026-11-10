use roundhouse::prelude::*;

use crate::domain::Profile;
use crate::error::ApiError;

routes! {
    prefix = "/profiles",
    tags = ["profiles"],
    "/" => { post(create) },
    "/unchecked" => { post(unchecked,
        validate = false,) },
}

/// @summary: Create a profile
#[handler]
pub async fn create(
    Strict(axum::Json(profile)): Strict<axum::Json<Profile>>,
) -> Result<Json<Profile>, ApiError> {
    if profile.username == "root" {
        return Err(ApiError::UsernameReserved {
            username: profile.username,
        });
    }
    Ok(Json(profile))
}

/// The same extractor, with `validate = false` on the table entry. This still deserializes and
/// still rejects malformed JSON; it only skips the `#[schema(..)]` constraint checks.
/// @summary: Create a profile without constraint checks
#[handler]
pub async fn unchecked(Strict(axum::Json(profile)): Strict<axum::Json<Profile>>) -> Json<Profile> {
    Json(profile)
}
