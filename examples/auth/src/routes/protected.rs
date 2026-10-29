use axum::http::StatusCode;
use roundhouse::prelude::*;
use serde::Serialize;

use crate::schemes::{ApiKeyAuth, BearerAuth, ReadPosts, WritePosts};

routes! {
    get("/public", public, summary = "No auth"),
    get("/private", private, summary = "Bearer required"),
    get("/scoped", scoped, summary = "Bearer with read scope"),
    get("/write", write, summary = "Bearer with write scope"),
    get("/optional", optional, summary = "Auth is optional"),
    get("/admin", admin, summary = "Bearer AND API key, admin only"),
    get("/projects/{project_id}", project, summary = "Resource policy after the scope gate"),
}

#[derive(Serialize, ApiSchema)]
pub struct Message {
    pub message: String,
}

#[handler]
pub async fn public() -> Json<Message> {
    Json(Message {
        message: "public".to_string(),
    })
}

#[handler]
pub async fn private(Auth { principal, .. }: Auth<BearerAuth>) -> Json<Message> {
    let _ = principal;
    Json(Message {
        message: "private".to_string(),
    })
}

#[handler]
pub async fn scoped(Auth { .. }: Auth<BearerAuth, ReadPosts>) -> Json<Message> {
    Json(Message {
        message: "read scope granted".to_string(),
    })
}

#[handler]
pub async fn write(Auth { .. }: Auth<BearerAuth, WritePosts>) -> Json<Message> {
    Json(Message {
        message: "write scope granted".to_string(),
    })
}

#[handler]
pub async fn optional(auth: Option<Auth<BearerAuth>>) -> Json<Message> {
    Json(Message {
        message: match auth {
            Some(_) => "authenticated".to_string(),
            None => "anonymous".to_string(),
        },
    })
}

#[allow(clippy::result_large_err)]
#[handler]
pub async fn admin(
    Auth { principal, .. }: Auth<BearerAuth>,
    _api_key: Auth<ApiKeyAuth>,
) -> Result<Json<Message>, Problem> {
    // AuthScheme::authenticate accepted the token; the role check is handler policy.
    if !principal.admin {
        return Err(Problem::new(StatusCode::FORBIDDEN, "Forbidden")
            .type_uri("https://roundhouse.rs/problems/Forbidden")
            .detail("admin role required"));
    }
    Ok(Json(Message {
        message: format!("admin {}", principal.name),
    }))
}

#[derive(serde::Deserialize, PathParams)]
pub struct ProjectPath {
    pub project_id: String,
}

/// `read:posts` is the route-level gate. Whether this user may read *this* project is resource
/// policy, so it is checked here with the typed path and the identity.
#[allow(clippy::result_large_err)]
#[handler]
pub async fn project(
    Auth { principal, .. }: Auth<BearerAuth, ReadPosts>,
    Path(ProjectPath { project_id }): Path<ProjectPath>,
) -> Result<Json<Message>, Problem> {
    if !principal
        .projects
        .iter()
        .any(|project| project == &project_id)
    {
        return Err(Problem::new(StatusCode::FORBIDDEN, "Forbidden")
            .type_uri("https://roundhouse.rs/problems/Forbidden")
            .detail(format!(
                "{} is not a member of {project_id}",
                principal.name
            )));
    }
    Ok(Json(Message {
        message: format!("{}/{}", principal.name, project_id),
    }))
}
