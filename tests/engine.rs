//! Schema execution and the conformance checks behind §7.7.

#![cfg(feature = "schema-engine")]

use axum::http::StatusCode;
use roundhouse::prelude::*;
use roundhouse::schema::engine;
use roundhouse::{App, Built, Validation};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct State;

#[derive(Debug, Deserialize, ToSchema, ApiSchema)]
#[serde(deny_unknown_fields)]
pub struct Note {
    #[schema(min_length = 1, max_length = 5)]
    pub title: String,
    #[schema(minimum = 0)]
    pub views: i64,
    #[schema(pattern = r"^[a-z-]+$")]
    pub slug: String,
}

fn check_value(value: &String) -> Result<(), ValidationErrors> {
    if value == "ok" {
        Ok(())
    } else {
        let mut errors = ValidationErrors::new();
        errors.push(ValidationError::new(
            "",
            "validate_with",
            "value must be `ok`",
        ));
        Err(errors)
    }
}

#[derive(Debug, Deserialize, ToSchema, ApiSchema)]
pub struct Checked {
    #[roundhouse(validate_with = check_value)]
    pub value: String,
}

#[derive(Debug, Deserialize, ToSchema, ApiSchema)]
pub struct Link {
    #[schema(format = "url")]
    pub url: String,
}

#[derive(Debug, Serialize, ToSchema, ApiSchema)]
pub struct NoteOut {
    pub ok: bool,
}

fn module() -> &'static str {
    concat!(module_path!(), "::schema_mode")
}

fn app(validation: Validation) -> Built {
    App::new(State)
        .scope(module())
        .validation(validation)
        .openapi(|| roundhouse::openapi::Info::new("Engine", "0.1.0"))
        .build()
}

mod lying {
    use super::*;

    mount!("/lying");

    routes! {
        get("/x", x, operation_id = "lying_x"),
    }

    #[derive(Serialize, ToSchema, ApiSchema)]
    pub struct Out {
        #[schema(min_length = 3)]
        pub name: String,
    }

    #[handler]
    pub async fn x() -> Json<Out> {
        Json(Out {
            name: "x".to_string(),
        })
    }
}

mod schema_mode {
    use super::*;

    mount!("/schema");

    routes! {
        post("/notes", create, operation_id = "schema_create_note"),
        post("/notes/bulk", create_bulk, operation_id = "schema_create_bulk"),
    }

    #[handler]
    pub async fn create(Json(note): Json<Note>) -> Json<NoteOut> {
        Json(NoteOut { ok: note.views > 0 })
    }

    #[handler]
    pub async fn create_bulk(Json(notes): Json<Vec<Note>>) -> Json<NoteOut> {
        Json(NoteOut {
            ok: !notes.is_empty(),
        })
    }
}

#[tokio::test]
async fn schema_mode_rejects_a_constraint_violation_with_a_pointer() {
    let app = app(Validation::Schema);
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/schema/notes",
            r#"{"title":"","views":1,"slug":"ok"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["errors"][0]["pointer"], "/title");
    assert_eq!(value["errors"][0]["code"], "min_length");
}

#[tokio::test]
async fn schema_mode_accepts_a_valid_body() {
    let app = app(Validation::Schema);
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/schema/notes",
            r#"{"title":"a","views":1,"slug":"a-b"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn schema_mode_rejects_unknown_properties() {
    let app = app(Validation::Schema);
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/schema/notes",
            r#"{"title":"a","views":1,"slug":"a-b","extra":true}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn schema_mode_compiles_composite_bodies() {
    let app = app(Validation::Schema);
    // The body is `Json<Vec<Note>>`: it has no component of its own, so the engine must compile
    // the operation's inline array schema rather than silently skipping validation.
    let document = serde_json::to_value(app.document()).unwrap();
    assert_eq!(
        document["paths"]["/schema/notes/bulk"]["post"]["requestBody"]["content"]["application/json"]
            ["schema"]["type"],
        "array"
    );

    let response = app
        .oneshot_json(
            http::Method::POST,
            "/schema/notes/bulk",
            r#"[{"title":"","views":1,"slug":"ok"}]"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .oneshot_json(
            http::Method::POST,
            "/schema/notes/bulk",
            r#"[{"title":"a","views":1,"slug":"a-b"}]"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn off_disables_constraint_checks() {
    let app = app(Validation::Off);
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/schema/notes",
            r#"{"title":"","views":1,"slug":"ok"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn response_validation_uses_the_engine_keywords() {
    let app = App::new(State)
        .scope(concat!(module_path!(), "::lying"))
        .openapi(|| roundhouse::openapi::Info::new("Engine", "0.1.0"))
        .validate_responses()
        .build();
    let response = app.oneshot_get("/lying/x").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        value["detail"]
            .as_str()
            .unwrap()
            .contains("response validation failed")
    );
}

#[test]
fn validate_with_is_published_and_runs_after_the_schema() {
    let mut components = roundhouse::utoipa::openapi::schema::Components::new();
    Checked::register(&mut components);
    let schema = serde_json::to_value(&components.schemas["Checked"]).unwrap();
    assert_eq!(schema["properties"]["value"]["x-validate-with"], true);

    let validator = engine::compile::<Checked>().expect("the emitted schema is valid");
    assert!(engine::is_valid(
        &validator,
        &serde_json::json!({"value": "not ok"}),
    ));
    let checked = Checked {
        value: "not ok".to_string(),
    };
    assert!(checked.validate().is_err());
}

#[test]
fn url_is_emitted_as_uri_and_enforced() {
    let validator = engine::compile::<Link>().expect("the emitted schema is valid");
    assert!(engine::is_valid(
        &validator,
        &serde_json::json!({"url": "https://example.com"}),
    ));
    assert!(!engine::is_valid(
        &validator,
        &serde_json::json!({"url": "not a url"}),
    ));
}

#[test]
fn the_engine_reports_stable_codes_and_pointers() {
    let validator = engine::compile::<Note>().expect("the emitted schema is valid");

    let fixtures = [
        (
            "empty title",
            r#"{"title":"","views":1,"slug":"a-b"}"#,
            "/title",
            "min_length",
        ),
        (
            "long title",
            r#"{"title":"too long","views":1,"slug":"a-b"}"#,
            "/title",
            "max_length",
        ),
        (
            "bad slug",
            r#"{"title":"ok","views":1,"slug":"A B"}"#,
            "/slug",
            "pattern",
        ),
        (
            "negative views",
            r#"{"title":"ok","views":-1,"slug":"a-b"}"#,
            "/views",
            "minimum",
        ),
    ];

    for (name, fixture, pointer, code) in fixtures {
        let value: serde_json::Value = serde_json::from_str(fixture).unwrap();
        let errors = engine::validate_with(&validator, &value)
            .expect_err(name)
            .into_vec();
        assert!(
            errors
                .iter()
                .any(|error| error.pointer == pointer && error.code == code),
            "fixture `{name}` did not report {code} at {pointer}: {errors:?}"
        );
    }

    let valid: serde_json::Value =
        serde_json::from_str(r#"{"title":"ok","views":1,"slug":"a-b"}"#).unwrap();
    assert!(engine::validate_with(&validator, &valid).is_ok());
}
