use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use roundhouse::Built;
use validation_example::{AppState, app};

fn built() -> Built {
    app(AppState).build()
}

async fn send(app: &Built, uri: &str, body: &str) -> axum::response::Response {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.oneshot(request).await
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

const VALID: &str = r#"{
    "username": "ada_lovelace",
    "email": "ada@example.com",
    "website": "https://example.com",
    "id": "123e4567-e89b-12d3-a456-426614174000",
    "age": 36,
    "score": 4.5,
    "tags": ["math", "code"],
    "address": { "street": "Main Street", "postalCode": "12345" },
    "nickname": null,
    "role": "author"
}"#;

const INVALID: &str = r#"{
    "username": "AB",
    "email": "not-an-email",
    "website": "not-a-url",
    "id": "not-a-uuid",
    "age": 17,
    "score": 10.0,
    "tags": ["x", "x"],
    "address": { "street": "A", "postalCode": "1" },
    "role": "author"
}"#;

#[tokio::test]
async fn a_valid_profile_round_trips() {
    let app = built();
    let response = send(&app, "/profiles", VALID).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value = body_json(response).await;
    assert_eq!(value["username"], "ada_lovelace");
    assert_eq!(value["address"]["postalCode"], "12345");
}

#[tokio::test]
async fn every_violation_is_collected_with_its_pointer() {
    let app = built();
    let response = send(&app, "/profiles", INVALID).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
    let value = body_json(response).await;
    let errors = value["errors"].as_array().unwrap();
    let has = |pointer: &str, code: &str| {
        errors
            .iter()
            .any(|error| error["pointer"] == pointer && error["code"] == code)
    };

    assert!(has("/username", "min_length"));
    assert!(has("/username", "pattern"));
    assert!(has("/email", "format"));
    assert!(has("/website", "format"));
    assert!(has("/id", "format"));
    assert!(has("/age", "minimum"));
    assert!(has("/score", "exclusive_maximum"));
    assert!(has("/tags", "unique_items"));
    assert!(has("/address/street", "min_length"));
    assert!(has("/address/postalCode", "pattern"));
}

#[tokio::test]
async fn validate_false_skips_constraints_but_not_deserialization() {
    let app = built();
    let response = send(&app, "/profiles/unchecked", INVALID).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value = body_json(response).await;
    assert_eq!(value["username"], "AB");

    let response = send(&app, "/profiles/unchecked", "{not json").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unknown_fields_are_rejected_by_serde() {
    let app = built();
    let body = VALID.replace("\"role\"", "\"extra\": true, \"role\"");
    let response = send(&app, "/profiles", &body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/problem+json"
    );
}

#[tokio::test]
async fn domain_errors_use_the_same_problem_shape() {
    let app = built();
    let body = VALID.replace("ada_lovelace", "root");
    let response = send(&app, "/profiles", &body).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let value = body_json(response).await;
    assert_eq!(
        value["type"],
        "https://roundhouse.rs/problems/UsernameReserved"
    );
    assert_eq!(value["detail"], "username `root` is reserved");
}

#[tokio::test]
async fn document_carries_the_constraints() {
    let app = built();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let profile = &document["components"]["schemas"]["Profile"];

    assert_eq!(profile["additionalProperties"], false);
    assert_eq!(profile["properties"]["username"]["minLength"], 3);
    assert_eq!(profile["properties"]["username"]["maxLength"], 32);
    assert_eq!(
        profile["properties"]["username"]["pattern"],
        "^[a-z0-9_-]+$"
    );
    assert_eq!(profile["properties"]["username"]["example"], "ada_lovelace");
    assert_eq!(profile["properties"]["email"]["format"], "email");
    assert_eq!(profile["properties"]["website"]["format"], "url");
    assert_eq!(profile["properties"]["id"]["format"], "uuid");
    assert_eq!(profile["properties"]["age"]["minimum"], 18);
    assert_eq!(profile["properties"]["age"]["maximum"], 130);
    assert_eq!(profile["properties"]["score"]["exclusiveMinimum"], 0);
    assert_eq!(profile["properties"]["score"]["exclusiveMaximum"], 10);
    assert_eq!(profile["properties"]["tags"]["uniqueItems"], true);
    assert_eq!(profile["properties"]["tags"]["minItems"], 1);
    assert_eq!(profile["properties"]["tags"]["maxItems"], 5);
    assert_eq!(
        profile["properties"]["address"]["$ref"],
        "#/components/schemas/Address"
    );
    assert_eq!(
        profile["required"],
        serde_json::json!([
            "username", "email", "id", "age", "score", "tags", "address", "role"
        ])
    );
    assert_eq!(
        document["components"]["schemas"]["Role"]["enum"],
        serde_json::json!(["reader", "author", "admin"])
    );
    assert_eq!(
        document["paths"]["/profiles"]["post"]["requestBody"]["content"]["application/json"]["schema"]
            ["$ref"],
        "#/components/schemas/Profile"
    );
}
