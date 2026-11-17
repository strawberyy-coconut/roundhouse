//! The glass-box rule: a type name must not carry documentation meaning.
//!
//! These tests shadow the built-in response names with local types that implement
//! `DocumentedResponse` differently. The document must follow the impls, not the names.

use roundhouse::doc::{ApiSchema, DocumentedResponse, ResponseDoc, SchemaContext};
use roundhouse::prelude::*;

roundhouse::app_state!(());

mod shadow {
    use super::*;

    #[roundhouse::schema]
    #[derive(serde::Serialize)]
    pub struct Note {
        pub title: String,
    }

    /// A local `Json` with a deliberately different status and content type.
    pub struct Json<T>(pub T);

    impl<T: serde::Serialize> axum::response::IntoResponse for Json<T> {
        fn into_response(self) -> axum::response::Response {
            (
                axum::http::StatusCode::from_u16(299).expect("299 is a valid status"),
                axum::Json(self.0),
            )
                .into_response()
        }
    }

    impl<T: ApiSchema> DocumentedResponse for Json<T> {
        fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
            T::register(context);
            let mut response = ResponseDoc::json(299, T::schema());
            response.content_type = Some("application/x-custom-json".to_string());
            response.description = "Custom JSON".to_string();
            vec![response]
        }
    }

    /// A local `Problem` that documents a success, not a problem. The name must not decide.
    pub struct Problem;

    impl axum::response::IntoResponse for Problem {
        fn into_response(self) -> axum::response::Response {
            (axum::http::StatusCode::OK, "not a problem").into_response()
        }
    }

    impl DocumentedResponse for Problem {
        fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
            vec![ResponseDoc::new(200, "Not a problem")]
        }
    }

    routes! {
        prefix = "/shadow",
        "/json" => { get(json, operation_id = "shadow_json") },
        "/problem" => { get(problem, operation_id = "shadow_problem") },
        "/declared" => { get(declared, operation_id = "shadow_declared") },
    }

    #[handler]
    pub async fn json() -> Json<Note> {
        Json(Note {
            title: "hello".to_string(),
        })
    }

    #[handler]
    pub async fn problem() -> Problem {
        Problem
    }

    #[handler(response(418, Problem))]
    pub async fn declared() {}
}

fn app() -> roundhouse::Built {
    roundhouse::App::new(())
        .include(&shadow::MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Glass box", "0.1.0"))
        .build()
}

#[test]
fn local_return_type_names_follow_their_impls() {
    let document = serde_json::to_value(app().document()).unwrap();

    let json = &document["paths"]["/shadow/json"]["get"]["responses"]["299"];
    assert!(json.is_object(), "{json}");
    assert_eq!(json["description"], "Custom JSON");
    assert_eq!(
        json["content"]["application/x-custom-json"]["schema"]["$ref"],
        "#/components/schemas/Note"
    );
    assert!(
        document["paths"]["/shadow/json"]["get"]["responses"]["200"].is_null(),
        "the name `Json` must not imply a 200 response"
    );

    let problem = &document["paths"]["/shadow/problem"]["get"]["responses"]["200"];
    assert!(problem.is_object(), "{problem}");
    assert_eq!(problem["description"], "Not a problem");
    assert!(
        document["paths"]["/shadow/problem"]["get"]["responses"]["418"].is_null(),
        "the name `Problem` must not imply a 4xx response"
    );
}

#[test]
fn declared_types_follow_their_impls_too() {
    let document = serde_json::to_value(app().document()).unwrap();
    let declared = &document["paths"]["/shadow/declared"]["get"]["responses"]["418"];
    assert!(declared.is_object(), "{declared}");
    assert_eq!(declared["description"], "Not a problem");
}
