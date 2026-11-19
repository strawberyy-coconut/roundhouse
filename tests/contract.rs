use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::{Method, StatusCode, header};
use roundhouse::openapi::Info;
use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

/// No `Default` implementation: the contract builder must not need a state value.
#[derive(Clone)]
pub struct AppState;

roundhouse::app_state!(AppState);

roundhouse::api! {
    state = AppState,
    title = "Contract Test",
    version = "0.1.0",
    root = crate::notes::MODULE,
    register = [Registered],
    response_defaults = {
        500 => "Unexpected server error.",
    },
}

#[roundhouse::schema]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub title: String,
}

#[roundhouse::schema]
#[derive(Clone, Debug, Serialize)]
pub struct Registered {
    pub value: String,
}

mod notes {
    use super::*;

    routes! {
        prefix = "/notes",
        tags = ["notes"],
        "/" => {
            get(list, operation_id = "list"),
            post(create, operation_id = "create"),
        },
        "/{id}" => { get(show, operation_id = "show") },
    }

    /// @response 200: All stored notes.
    #[handler]
    pub async fn list(State(_): State<AppState>) -> Json<Vec<Note>> {
        Json(Vec::new())
    }

    #[handler]
    pub async fn create(State(_): State<AppState>, Json(body): Json<Note>) -> Json<Note> {
        Json(body)
    }

    #[handler]
    pub async fn show(State(_): State<AppState>, Path(id): Path<i64>) -> Json<Note> {
        Json(Note {
            id,
            title: "example".to_string(),
        })
    }
}

mod verify {
    use super::*;

    routes! {
        "/verify-mismatch" => { get(mismatch, operation_id = "verify_mismatch") },
    }

    #[handler(response(200, Json<Note>))]
    pub async fn mismatch() -> Response {
        Json(Registered {
            value: "not a note".to_string(),
        })
        .into_response()
    }
}

routes! {
    ...notes,
}

fn contract() -> roundhouse::BuiltContract {
    __roundhouse_contract().build().expect("contract builds")
}

fn bundle(contract: &roundhouse::BuiltContract) -> roundhouse::ContractBundle {
    roundhouse::ContractBundle::new(
        Box::leak(
            contract
                .contract_json()
                .expect("contract serializes")
                .into_boxed_str(),
        ),
        Box::leak(contract.openapi_json().into_boxed_str()),
    )
}

#[test]
fn contract_records_every_operation_and_request_body() {
    let contract = contract();
    assert!(contract.diagnostics().is_empty());

    let json: serde_json::Value =
        serde_json::from_str(&contract.contract_json().expect("contract serializes"))
            .expect("contract is JSON");

    assert_eq!(json["kind"], "roundhouse.contract");
    assert_eq!(json["contract_version"], 1);
    assert_eq!(json["openapi"], "openapi.json");

    let operations = json["operations"].as_array().expect("operations array");
    assert_eq!(operations.len(), 3);

    let create = operations
        .iter()
        .find(|operation| operation["operation_id"] == "create")
        .expect("create operation");
    assert_eq!(create["method"], "POST");
    assert_eq!(create["path"], "/notes");
    assert_eq!(
        create["request"]["body"]["content_type"],
        "application/json"
    );
    assert!(
        create["request"]["body"]["schema"]["$ref"]
            .as_str()
            .is_some_and(|reference| reference.ends_with("/Note"))
    );

    let openapi: serde_json::Value =
        serde_json::from_str(&contract.openapi_json()).expect("openapi is JSON");
    assert!(openapi["paths"]["/notes"].is_object());
    assert!(openapi["paths"]["/notes/{id}"].is_object());
    assert_eq!(
        openapi["paths"]["/notes"]["get"]["responses"]["200"]["description"],
        "All stored notes."
    );
}

#[tokio::test]
async fn static_contract_is_served_verbatim() {
    let contract = contract();
    let openapi: &'static str = Box::leak(contract.openapi_json().into_boxed_str());
    let contract_json: &'static str = Box::leak(
        contract
            .contract_json()
            .expect("contract serializes")
            .into_boxed_str(),
    );
    let bundle = roundhouse::openapi::ContractBundle::new(contract_json, openapi);

    let built = roundhouse::App::new(AppState)
        .fragment(__roundhouse_fragment())
        .openapi(|| roundhouse::openapi::Info::new("Contract Test", "0.1.0"))
        .contract(bundle)
        .build();

    let response = built.oneshot_get("/openapi.json").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE].to_str().unwrap(),
        "application/json"
    );
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(std::str::from_utf8(&body).expect("utf8"), openapi);
}

#[test]
fn an_unsupported_contract_version_is_rejected() {
    let contract = contract();
    let stale = contract
        .contract_json()
        .expect("contract serializes")
        .replace("\"contract_version\": 1", "\"contract_version\": 99");
    let openapi = Box::leak(contract.openapi_json().into_boxed_str());
    let stale = Box::leak(stale.into_boxed_str());

    let error = match roundhouse::App::new(AppState)
        .fragment(__roundhouse_fragment())
        .openapi(|| roundhouse::openapi::Info::new("Contract Test", "0.1.0"))
        .contract(roundhouse::ContractBundle::new(stale, openapi))
        .try_build()
    {
        Ok(_) => panic!("a future contract version must not be accepted"),
        Err(error) => error,
    };
    assert!(
        matches!(error, roundhouse::BuildError::InvalidContract { .. }),
        "{error}"
    );
}

#[tokio::test]
async fn contract_validation_guards_the_request_body() {
    let contract = contract();
    let openapi: &'static str = Box::leak(contract.openapi_json().into_boxed_str());
    let contract_json: &'static str = Box::leak(
        contract
            .contract_json()
            .expect("contract serializes")
            .into_boxed_str(),
    );
    let bundle = roundhouse::ContractBundle::new(contract_json, openapi);

    let built = roundhouse::App::new(AppState)
        .fragment(__roundhouse_fragment())
        .openapi(|| roundhouse::openapi::Info::new("Contract Test", "0.1.0"))
        .contract(bundle)
        .build();

    let invalid = built.oneshot_json(Method::POST, "/notes", "{}").await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let valid = built
        .oneshot_json(Method::POST, "/notes", r#"{"id":1,"title":"hello"}"#)
        .await;
    assert_eq!(valid.status(), StatusCode::OK);
}

#[tokio::test]
async fn contract_response_validator_checks_the_actual_response() {
    let contract = contract();
    let contract_json = contract.contract_json().expect("contract serializes");
    let validator = roundhouse::doc::ContractResponseValidator::from_contract_json(&contract_json)
        .expect("response plans compile");

    let openapi: &'static str = Box::leak(contract.openapi_json().into_boxed_str());
    let contract_json: &'static str = Box::leak(contract_json.into_boxed_str());
    let bundle = roundhouse::ContractBundle::new(contract_json, openapi);
    let built = roundhouse::App::new(AppState)
        .fragment(__roundhouse_fragment())
        .openapi(|| roundhouse::openapi::Info::new("Contract Test", "0.1.0"))
        .contract(bundle)
        .build();

    let response = built.oneshot_get("/notes/1").await;
    validator
        .validate("show", response)
        .await
        .expect("the documented response matches");

    assert!(
        validator
            .validate_value(
                "show",
                200,
                Some("application/json"),
                &serde_json::json!({ "id": "bad" })
            )
            .is_err()
    );
}

#[test]
fn contract_writes_both_artifacts() {
    let contract = contract();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "roundhouse-contract-test-{}-{unique}",
        std::process::id()
    ));

    contract.write_to(&directory).expect("contract writes");

    let contract_json =
        std::fs::read_to_string(directory.join("contract.json")).expect("contract.json exists");
    let openapi_json =
        std::fs::read_to_string(directory.join("openapi.json")).expect("openapi.json exists");
    serde_json::from_str::<serde_json::Value>(&contract_json).expect("contract.json parses");
    serde_json::from_str::<serde_json::Value>(&openapi_json).expect("openapi.json parses");
    assert!(contract_json.ends_with('\n'));
    assert!(openapi_json.ends_with('\n'));

    std::fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn contract_sources_are_relative_to_the_package_root() {
    let contract = contract();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let json: serde_json::Value = serde_json::from_str(
        &contract
            .contract_json_with_root(Some(root))
            .expect("contract serializes"),
    )
    .expect("contract is JSON");

    for operation in json["operations"].as_array().expect("operations array") {
        let file = operation["source"]["file"]
            .as_str()
            .expect("source file is a string");
        assert!(
            !file.starts_with('/'),
            "source path should be package-relative: {file}"
        );
    }
}

#[tokio::test]
async fn contract_verification_accepts_a_matching_response() {
    let contract = contract();
    let built = roundhouse::App::new(AppState)
        .fragment(__roundhouse_fragment())
        .contract(bundle(&contract))
        .verify_responses()
        .build();

    let response = built.oneshot_get("/notes/1").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn contract_verification_flags_a_mismatched_response() {
    let contract = roundhouse::Contract::<AppState>::new(Info::new("Verify", "0.1.0"))
        .include(&verify::MODULE)
        .build()
        .expect("verify contract builds");
    let built = roundhouse::App::new(AppState)
        .fragment(verify::__roundhouse_fragment())
        .contract(bundle(&contract))
        .verify_responses()
        .build();

    let response = built.oneshot_get("/verify-mismatch").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
