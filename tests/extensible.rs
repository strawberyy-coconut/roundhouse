//! Trait-based extensions: a custom axum extractor and a custom response type.

use axum::extract::FromRequestParts;
use axum::http::{StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use roundhouse::App;
use roundhouse::doc::{
    ApiResponse, ArgDoc, DocContext, ExtractArg, Param, ParamIn, ResponseDoc, SchemaContext, schema,
};
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

/// A normal axum extractor: the framework learns how to document it with one impl.
pub struct Tenant(pub String);

impl<S> FromRequestParts<S> for Tenant
where
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let tenant = parts
            .headers
            .get("x-tenant")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("anonymous");
        Ok(Self(tenant.to_string()))
    }
}

impl ExtractArg for Tenant {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            params: vec![
                Param::new("x-tenant", ParamIn::Header, schema::string()).description("tenant id"),
            ],
            ..ArgDoc::default()
        }
    }
}

/// A custom response type: implement `ApiResponse` and `routes!` documents it automatically.
pub struct Csv;

impl IntoResponse for Csv {
    fn into_response(self) -> Response {
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/csv")],
            "id,name\n",
        )
            .into_response()
    }
}

impl ApiResponse for Csv {
    const SUCCESS: &'static [u16] = &[200];

    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc {
            status: 200,
            description: "OK".to_string(),
            content_type: Some("text/csv".to_string()),
            schema: Some(schema::string()),
            headers: Vec::new(),
        }]
    }
}

routes! {
    prefix = "/extensible",
    "/tenant" => { get(tenant, operation_id = "extensible_tenant") },
    "/csv" => { get(csv, operation_id = "extensible_csv") },
}

#[handler]
pub async fn tenant(Tenant(tenant): Tenant) -> String {
    tenant
}

#[handler]
pub async fn csv() -> Csv {
    Csv
}

fn built() -> roundhouse::Built {
    App::new(State)
        .include(&MODULE)
        .openapi(|| roundhouse::openapi::Info::new("Extensible", "0.1.0"))
        .build()
}

#[test]
fn a_custom_extractor_documents_its_parameters() {
    let document = serde_json::to_value(built().document()).unwrap();
    let parameters = &document["paths"]["/extensible/tenant"]["get"]["parameters"];
    assert_eq!(
        parameters[0]["name"], "x-tenant",
        "the custom extractor's parameter is published: {parameters}"
    );
    assert_eq!(parameters[0]["in"], "header");
    assert_eq!(parameters[0]["description"], "tenant id");
}

#[test]
fn a_custom_response_documents_its_status_and_content_type() {
    let document = serde_json::to_value(built().document()).unwrap();
    let response = &document["paths"]["/extensible/csv"]["get"]["responses"]["200"];
    assert_eq!(response["content"]["text/csv"]["schema"]["type"], "string");
}

#[tokio::test]
async fn the_custom_extractor_still_runs() {
    let response = built()
        .oneshot(
            axum::http::Request::builder()
                .uri("/extensible/tenant")
                .header("x-tenant", "acme")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
}
