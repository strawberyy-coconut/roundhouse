//! App-level document metadata: servers, tag descriptions and the document hook.

use roundhouse::prelude::*;
use roundhouse::utoipa::openapi::Server;
use roundhouse::utoipa::openapi::tag::Tag;

#[derive(Clone, Default)]
pub struct State;

mod routes {
    use super::*;

    routes! {
        prefix = "/things",
        get("/", list, operation_id = "things_list", tags = ["things"]),
    }

    #[handler]
    pub async fn list() -> &'static str {
        "things"
    }
}

#[test]
fn servers_and_the_document_hook_reach_the_built_document() {
    let built = roundhouse::App::new(State)
        .include(&routes::MODULE)
        .server(Server::new("https://api.example.com"))
        .servers([Server::new("https://staging.example.com")])
        .document(|document| {
            document.as_utoipa_mut().info.description = Some("written by a hook".to_string());
            let mut tag = Tag::new("things");
            tag.description = Some("Everything about things".to_string());
            document.as_utoipa_mut().tags = Some(vec![tag]);
        })
        .build();

    let document = serde_json::to_value(built.document()).unwrap();
    assert_eq!(document["servers"][0]["url"], "https://api.example.com");
    assert_eq!(document["servers"][1]["url"], "https://staging.example.com");
    assert_eq!(document["info"]["description"], "written by a hook");
    assert_eq!(document["tags"][0]["name"], "things");
    assert_eq!(
        document["tags"][0]["description"],
        "Everything about things"
    );
}
