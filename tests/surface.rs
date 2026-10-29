use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use roundhouse::App;
use roundhouse::prelude::*;
use roundhouse::utoipa::openapi::Info;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default)]
pub struct State;

async fn marker(request: axum::extract::Request, next: Next) -> Response {
    let module = request
        .headers()
        .get("x-test")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("module")
        .to_string();
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::try_from(module) {
        response.headers_mut().insert("x-layer", value);
    }
    response
}

mod grouped {
    use super::*;

    mount!("/grouped", tags = ["grouped"]);

    routes! {
        "/" => {
            get(index, summary = "Index"),
            post(create),
        },
        "/{id}" => {
            get(show, operation_id = "grouped_show"),
            delete(destroy),
        },
    }

    #[handler]
    pub async fn index() -> &'static str {
        "index"
    }
    #[handler]
    pub async fn create() -> &'static str {
        "created"
    }
    #[handler]
    pub async fn show() -> &'static str {
        "shown"
    }
    #[handler]
    pub async fn destroy() -> &'static str {
        "destroyed"
    }
}

mod legacy {
    use super::*;

    mount!("/ignored");

    routes! {
        get("/legacy", legacy, absolute, tags = ["legacy"]),
    }

    #[handler]
    pub async fn legacy() -> &'static str {
        "legacy"
    }
}

mod gated {
    use super::*;

    mount!("/gated");

    routes! {
        get("/on", on),
        #[cfg(any())]
        get("/off", off),
    }

    #[handler]
    pub async fn on() -> &'static str {
        "on"
    }

    #[allow(dead_code)]
    #[handler]
    pub async fn off() -> &'static str {
        "off"
    }
}

mod layered {
    use super::*;

    mount!("/layered", layer = middleware::from_fn(marker));

    routes! {
        get("/ok", ok, operation_id = "layered_ok"),
        get("/entry", entry, layer = middleware::from_fn(marker)),
    }

    #[handler]
    pub async fn ok() -> &'static str {
        "ok"
    }
    #[handler]
    pub async fn entry() -> &'static str {
        "entry"
    }
}

mod route_layered {
    use super::*;

    mount!(
        "/route-layered",
        layer = middleware::from_fn(marker),
        layer_mode = route,
    );

    routes! { get("/ok", ok, operation_id = "route_layered_ok") }

    #[handler]
    pub async fn ok() -> &'static str {
        "ok"
    }
}

mod secured {
    use super::*;

    pub struct Bearer;

    impl AuthScheme for Bearer {
        type State = State;
        type Identity = ();

        const NAME: &'static str = "bearer";
        const KIND: roundhouse::SecuritySchemeKind = roundhouse::SecuritySchemeKind::HttpBearer;
    }

    impl Authenticate for Bearer {
        async fn authenticate(
            parts: &mut axum::http::request::Parts,
            _state: &State,
        ) -> Result<(), Problem> {
            if parts.headers.contains_key(header::AUTHORIZATION) {
                Ok(())
            } else {
                Err(Problem::new(StatusCode::UNAUTHORIZED, "Unauthorized"))
            }
        }

        fn presents_credentials(parts: &axum::http::request::Parts) -> bool {
            parts.headers.contains_key(header::AUTHORIZATION)
        }
    }

    mount!("/secured", security = ["bearer"]);

    routes! { get("/me", me) }

    #[handler]
    pub async fn me(_auth: Auth<Bearer>) -> &'static str {
        "me"
    }
}

mod plain {
    use super::*;

    mount!("/plain");

    routes! {
        get("/", get_plain),
        post("/", post_plain),
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct Plain {
        pub id: i64,
    }

    // Deliberately axum's Json: check 1 says an axum handler body compiles as written. `Plain`
    // does not implement ApiSchema, so the document simply has no inferred bodies here.
    #[handler]
    pub async fn get_plain() -> axum::Json<Plain> {
        axum::Json(Plain { id: 1 })
    }

    #[handler]
    pub async fn post_plain(body: axum::Json<Plain>) -> axum::Json<Plain> {
        body
    }
}

mod split_notes {
    use super::*;

    mount!("/split-notes", tags = ["split"]);

    pub mod list {
        use roundhouse::prelude::*;

        routes! {
            get("/", list, summary = "List split notes"),
        }

        #[handler]
        pub async fn list() -> &'static str {
            "list"
        }
    }

    pub mod show {
        use roundhouse::prelude::*;

        routes! {
            get("/{id}", show, operation_id = "split_show"),
        }

        #[handler]
        pub async fn show() -> &'static str {
            "show"
        }
    }
}

mod collision_a {
    use roundhouse::prelude::*;
    use serde::{Deserialize, Serialize};

    routes! { get("/collision-a", show, operation_id = "collision_a_show") }

    #[derive(Serialize, Deserialize, ApiSchema)]
    pub struct Collision {
        pub a: String,
    }

    #[handler]
    pub async fn show() -> Json<Collision> {
        Json(Collision { a: "a".to_string() })
    }
}

mod collision_b {
    use roundhouse::prelude::*;
    use serde::{Deserialize, Serialize};

    routes! { get("/collision-b", show, operation_id = "collision_b_show") }

    #[derive(Serialize, Deserialize, ApiSchema)]
    pub struct Collision {
        pub b: String,
    }

    #[handler]
    pub async fn show() -> Json<Collision> {
        Json(Collision { b: "b".to_string() })
    }
}

mod documented_layer {
    use roundhouse::prelude::*;

    mount!(
        "/documented",
        doc = ::roundhouse::doc::layer::rate_limit(429),
    );

    routes! { get("/ok", ok, operation_id = "documented_ok") }

    #[handler]
    pub async fn ok() -> &'static str {
        "ok"
    }
}

mod erased {
    use roundhouse::prelude::*;
    use std::sync::Arc;

    routes! { get("/erased", erased) }

    /// No `State<T>` anywhere: this is the erased-style handler shape, served with unit app state.
    #[handler]
    pub async fn erased(Extension(value): Extension<Arc<usize>>) -> String {
        format!("{value}")
    }
}

mod shapes {
    use super::*;

    mount!("/shapes");

    routes! {
        get("/color", color),
        get("/event", event),
        post("/constrained-event", constrained_event),
        post("/even", even),
        get("/utoipa", from_utoipa),
        post("/created", created),
        post("/contact", contact),
        post("/envelope", envelope),
        post("/unvalidated", unvalidated, validate = false),
        get("/node", node),
        get("/tree", tree),
        get("/lying", lying),
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    #[serde(rename_all = "kebab-case")]
    pub enum Color {
        Red,
        DarkBlue,
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Event {
        Created { id: i64 },
        Deleted { id: i64 },
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    pub struct Even {
        #[schema(validate_with = validate_even)]
        pub value: i64,
    }

    fn validate_even(value: &i64) -> Result<(), ::roundhouse::ValidationErrors> {
        if value % 2 == 0 {
            Ok(())
        } else {
            let mut errors = ::roundhouse::ValidationErrors::new();
            errors.push(::roundhouse::ValidationError::new(
                "",
                "even",
                "must be even",
            ));
            Err(errors)
        }
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    #[schema(via = utoipa)]
    pub struct FromUtoipa {
        pub name: String,
    }

    impl ::roundhouse::utoipa::PartialSchema for FromUtoipa {
        fn schema()
        -> ::roundhouse::utoipa::openapi::RefOr<::roundhouse::utoipa::openapi::schema::Schema>
        {
            ::roundhouse::utoipa::openapi::schema::Object::with_type(
                ::roundhouse::utoipa::openapi::schema::Type::String,
            )
            .into()
        }
    }

    impl ::roundhouse::utoipa::ToSchema for FromUtoipa {}

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum ConstrainedEvent {
        Created {
            #[schema(min_length = 3)]
            title: String,
        },
        Deleted,
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Note {
        pub id: i64,
        pub long_title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub subtitle: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(ApiResponse)]
    pub enum Created {
        #[status(201, description = "Created")]
        Created {
            #[body]
            body: Json<Note>,
            #[header("X-Id")]
            id: String,
            #[location]
            location: String,
        },
        #[status(409, problem = "AlreadyExists")]
        Conflict,
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    pub struct Contact {
        #[schema(format = "email")]
        pub email: String,
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    pub struct Nested {
        #[schema(min_length = 2)]
        pub name: String,
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    pub struct Envelope {
        pub nested: Nested,
        #[schema(min_items = 1, max_items = 2)]
        pub tags: Vec<String>,
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    pub struct Node {
        pub value: String,
        pub children: Vec<Node>,
    }

    #[derive(Debug, Serialize, Deserialize, ApiSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum Tree {
        Leaf { value: String },
        Branch { children: Vec<Tree> },
    }

    pub struct Lying;

    impl Serialize for Lying {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_i64(1)
        }
    }

    impl ::roundhouse::doc::ApiSchema for Lying {
        fn schema()
        -> ::roundhouse::utoipa::openapi::RefOr<::roundhouse::utoipa::openapi::schema::Schema>
        {
            ::roundhouse::doc::schema::string()
        }
    }

    #[handler]
    pub async fn lying() -> Json<Lying> {
        Json(Lying)
    }

    #[handler]
    pub async fn tree() -> Json<Tree> {
        Json(Tree::Branch {
            children: vec![Tree::Leaf {
                value: "leaf".to_string(),
            }],
        })
    }

    #[handler]
    pub async fn node() -> Json<Node> {
        Json(Node {
            value: "root".to_string(),
            children: Vec::new(),
        })
    }

    #[handler]
    pub async fn unvalidated(Json(_body): Json<Contact>) -> StatusCode {
        StatusCode::NO_CONTENT
    }

    #[handler]
    pub async fn envelope(Json(_body): Json<Envelope>) -> StatusCode {
        StatusCode::NO_CONTENT
    }

    #[handler]
    pub async fn contact(Json(_body): Json<Contact>) -> StatusCode {
        StatusCode::NO_CONTENT
    }

    #[handler]
    pub async fn color() -> Json<Color> {
        Json(Color::Red)
    }
    #[handler]
    pub async fn event() -> Json<Event> {
        Json(Event::Created { id: 1 })
    }
    #[handler]
    pub async fn constrained_event(Json(_event): Json<ConstrainedEvent>) -> StatusCode {
        StatusCode::NO_CONTENT
    }
    #[handler]
    pub async fn even(Json(_even): Json<Even>) -> StatusCode {
        StatusCode::NO_CONTENT
    }
    #[handler]
    pub async fn from_utoipa() -> Json<FromUtoipa> {
        Json(FromUtoipa {
            name: "external".to_string(),
        })
    }
    #[handler]
    pub async fn created() -> Created {
        Created::Created {
            body: Json(Note {
                id: 1,
                long_title: "hello".to_string(),
                subtitle: None,
            }),
            id: "1".to_string(),
            location: "/shapes/1".to_string(),
        }
    }
}

fn app() -> roundhouse::Built {
    App::new(State)
        .scope(module_path!())
        .openapi(|| Info::new("Surface", "0.1.0"))
        .build()
}

async fn get(app: &roundhouse::Built, uri: &str) -> Response {
    app.oneshot_get(uri).await
}

#[tokio::test]
async fn grouped_table_serves_every_method() {
    let app = app();
    assert_eq!(get(&app, "/grouped").await.status(), StatusCode::OK);
    assert_eq!(
        app.oneshot_json(http::Method::POST, "/grouped", "{}")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(get(&app, "/grouped/1").await.status(), StatusCode::OK);
    let request = Request::builder()
        .method(http::Method::DELETE)
        .uri("/grouped/1")
        .body(axum::body::Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(request).await.status(), StatusCode::OK);
    let request = Request::builder()
        .method(http::Method::PUT)
        .uri("/grouped")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET,HEAD,POST");
}

#[tokio::test]
async fn absolute_ignores_the_module_prefix() {
    let app = app();
    assert_eq!(get(&app, "/legacy").await.status(), StatusCode::OK);
    assert_eq!(
        get(&app, "/ignored/legacy").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn cfg_entries_vanish_with_their_gate() {
    let app = app();
    assert_eq!(get(&app, "/gated/on").await.status(), StatusCode::OK);
    assert_eq!(
        get(&app, "/gated/off").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn module_and_entry_layers_run_in_order() {
    let app = app();
    let response = Request::builder()
        .uri("/layered/ok")
        .header("x-test", "outer")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(response).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-layer"], "outer");

    let response = Request::builder()
        .uri("/layered/entry")
        .header("x-test", "entry")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = app.oneshot(response).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-layer"], "entry");

    // An unmatched path under a nested module falls through to the app fallback, which is
    // outside the module layer (axum's `nest` semantics).
    let response = get(&app, "/layered/missing").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get("x-layer").is_none());
}

#[tokio::test]
async fn route_layer_does_not_wrap_404s() {
    let app = app();
    let response = get(&app, "/route-layered/ok").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-layer"], "module");

    let response = get(&app, "/route-layered/missing").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(response.headers().get("x-layer").is_none());
}

#[tokio::test]
async fn security_from_mount_reaches_the_document() {
    let app = app();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let security = &document["paths"]["/secured/me"]["get"]["security"][0]["bearer"];
    assert!(security.is_array());
    assert_eq!(
        document["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
}

#[tokio::test]
async fn schema_derives_cover_enums_and_serde_names() {
    let app = app();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let color = &document["components"]["schemas"]["Color"];
    assert_eq!(color["type"], "string");
    assert_eq!(color["enum"][1], "dark-blue");

    let event = &document["components"]["schemas"]["Event"];
    assert!(event["oneOf"].as_array().unwrap().len() >= 2);

    let note = &document["components"]["schemas"]["Note"];
    assert!(note["properties"]["longTitle"].is_object());
    assert_eq!(note["additionalProperties"], false);
    assert!(
        note["properties"]["subtitle"]["type"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("null"))
    );
    assert_eq!(note["required"], serde_json::json!(["id", "longTitle"]));
}

#[tokio::test]
async fn api_response_enum_sets_status_body_and_headers() {
    let app = app();
    let response = app
        .oneshot_json(http::Method::POST, "/shapes/created", "{}")
        .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()["x-id"], "1");
    assert_eq!(response.headers()[header::LOCATION], "/shapes/1");
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");

    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let responses = &document["paths"]["/shapes/created"]["post"]["responses"];
    assert!(responses["201"].is_object());
    assert!(responses["409"].is_object());
    assert_eq!(
        responses["201"]["headers"]["X-Id"]["schema"]["type"],
        "string"
    );
    assert_eq!(
        responses["201"]["headers"]["Location"]["schema"]["format"],
        "uri"
    );
}

#[tokio::test]
async fn document_rejections_can_be_suppressed() {
    let app = App::new(State)
        .scope(module_path!())
        .document_rejections(false)
        .openapi(|| Info::new("Surface", "0.1.0"))
        .build();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let responses = &document["paths"]["/shapes/created"]["post"]["responses"];
    assert!(responses.get("400").is_none());
    assert!(responses.get("415").is_none());
}

#[tokio::test]
async fn app_merge_and_custom_fallback() {
    let merged = axum::Router::new().route("/merged", axum::routing::get(|| async { "merged" }));
    let app = App::new(State)
        .scope(module_path!())
        .merge(merged)
        .fallback(|| async { (StatusCode::IM_A_TEAPOT, "custom") })
        .build();
    let response = get(&app, "/merged").await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = get(&app, "/missing").await;
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);
}

#[tokio::test]
async fn scope_and_include_select_roots() {
    let app = App::new(State)
        .scope(concat!(module_path!(), "::grouped"))
        .include(concat!(module_path!(), "::legacy"))
        .build();
    assert_eq!(get(&app, "/grouped").await.status(), StatusCode::OK);
    assert_eq!(get(&app, "/legacy").await.status(), StatusCode::OK);
    assert_eq!(
        get(&app, "/layered/ok").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn axum_json_handlers_compile_and_serve_without_schemas() {
    let app = app();
    assert_eq!(get(&app, "/plain").await.status(), StatusCode::OK);
    let response = app
        .oneshot_json(http::Method::POST, "/plain", r#"{"id":7}"#)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn format_constraints_are_documented_and_enforced() {
    let app = app();
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/contact",
            r#"{"email":"not-an-email"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["errors"][0]["pointer"], "/email");
    assert_eq!(value["errors"][0]["code"], "format");

    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/contact",
            r#"{"email":"a@example.com"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    assert_eq!(
        document["components"]["schemas"]["Contact"]["properties"]["email"]["format"],
        "email"
    );
}

#[tokio::test]
async fn nested_and_collection_violations_arrive_together() {
    let app = app();
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/envelope",
            r#"{"nested":{"name":"x"},"tags":[]}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let errors = value["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error["pointer"] == "/nested/name" && error["code"] == "min_length")
    );
    assert!(
        errors
            .iter()
            .any(|error| error["pointer"] == "/tags" && error["code"] == "min_items")
    );
}

#[tokio::test]
async fn recursive_schemas_use_a_reference_instead_of_looping() {
    let app = app();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    assert_eq!(
        document["components"]["schemas"]["Node"]["properties"]["children"]["items"]["$ref"],
        "#/components/schemas/Node"
    );
}

#[tokio::test]
async fn validate_false_opts_the_route_out_of_constraint_checks() {
    let app = app();
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/unvalidated",
            r#"{"email":"not-an-email"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn route_tables_in_leaf_modules_inherit_the_parent_mount() {
    let app = app();
    assert_eq!(get(&app, "/split-notes").await.status(), StatusCode::OK);
    assert_eq!(get(&app, "/split-notes/7").await.status(), StatusCode::OK);

    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    assert_eq!(
        document["paths"]["/split-notes"]["get"]["tags"],
        serde_json::json!(["split"])
    );
    assert_eq!(
        document["paths"]["/split-notes/{id}"]["get"]["tags"],
        serde_json::json!(["split"])
    );
}

#[tokio::test]
async fn validation_status_is_configurable() {
    let app = App::new(State)
        .scope(module_path!())
        .validation_status(StatusCode::UNPROCESSABLE_ENTITY)
        .openapi(|| Info::new("Surface", "0.1.0"))
        .build();
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/contact",
            r#"{"email":"not-an-email"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["status"], 422);
}

#[tokio::test]
async fn enum_variant_fields_are_validated() {
    let app = app();
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/constrained-event",
            r#"{"kind":"created","title":"x"}"#,
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

    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/constrained-event",
            r#"{"kind":"created","title":"long enough"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn validate_with_runs_next_to_the_declared_constraints() {
    let app = app();
    let response = app
        .oneshot_json(http::Method::POST, "/shapes/even", r#"{"value":3}"#)
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["errors"][0]["pointer"], "/value");
    assert_eq!(value["errors"][0]["code"], "even");

    let response = app
        .oneshot_json(http::Method::POST, "/shapes/even", r#"{"value":4}"#)
        .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn schema_can_delegate_to_utoipa() {
    let app = app();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    assert_eq!(
        document["paths"]["/shapes/utoipa"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["type"],
        "string"
    );
}

#[tokio::test]
async fn colliding_component_names_get_their_module_path() {
    let app = app();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let schemas = &document["components"]["schemas"];
    assert!(schemas["surface.collision_a.Collision"].is_object());
    assert!(schemas["surface.collision_b.Collision"].is_object());
    assert_eq!(
        document["paths"]["/collision-a"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"]["$ref"],
        "#/components/schemas/surface.collision_a.Collision"
    );
}

#[tokio::test]
async fn declared_layer_docs_merge_into_the_operation() {
    let app = app();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let responses = &document["paths"]["/documented/ok"]["get"]["responses"];
    assert!(responses["429"].is_object());
    assert_eq!(
        responses["429"]["headers"]["Retry-After"]["schema"]["type"],
        "integer"
    );
}

#[tokio::test]
async fn swagger_ui_is_served_when_requested() {
    let app = App::new(State)
        .scope(module_path!())
        .openapi(|| Info::new("Surface", "0.1.0"))
        .docs()
        .build();
    let response = app.oneshot_get("/docs").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("swagger-ui"));
    assert!(html.contains("/openapi.json"));
}

#[tokio::test]
async fn documents_can_be_downgraded_to_openapi_30() {
    let app = app();
    let value = app.openapi_30().expect("openapi enabled");
    assert_eq!(value["openapi"], "3.0.3");
    let event = &value["components"]["schemas"]["Note"];
    assert_eq!(event["properties"]["subtitle"]["type"], "string");
    assert_eq!(event["properties"]["subtitle"]["nullable"], true);
}

#[tokio::test]
async fn erased_style_handlers_use_unit_state_and_extensions() {
    let app = App::new(State).scope(module_path!()).build();
    let router = app
        .router()
        .layer(axum::Extension(std::sync::Arc::new(7usize)));
    let request = Request::builder()
        .uri("/erased")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = tower::ServiceExt::oneshot(router, request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(&body[..], b"7");
}

#[tokio::test]
async fn complex_recursive_enums_terminate() {
    let app = app();
    let document = serde_json::to_value(app.document().unwrap()).unwrap();
    let tree = &document["components"]["schemas"]["Tree"];
    assert!(tree["oneOf"].is_array());
    assert_eq!(
        tree["oneOf"][1]["properties"]["children"]["items"]["$ref"],
        "#/components/schemas/Tree"
    );
}

#[cfg(feature = "schema-engine")]
#[tokio::test]
async fn response_validation_catches_a_document_that_lies() {
    let app = App::new(State)
        .scope(module_path!())
        .openapi(|| Info::new("Surface", "0.1.0"))
        .validate_responses()
        .build();

    let response = app.oneshot_get("/shapes/color").await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = app.oneshot_get("/shapes/lying").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["title"], "Internal Server Error");
    assert!(
        value["detail"]
            .as_str()
            .unwrap()
            .contains("response validation failed")
    );
}

#[tokio::test]
async fn max_body_is_a_global_limit() {
    let app = App::new(State).scope(module_path!()).max_body(8).build();
    let response = app
        .oneshot_json(
            http::Method::POST,
            "/shapes/contact",
            r#"{"email":"a@example.com"}"#,
        )
        .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
