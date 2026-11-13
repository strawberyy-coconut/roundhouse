//! Request-path throughput, without a benchmark framework.
//!
//! Run with:
//!
//! ```sh
//! cargo bench -p roundhouse --bench throughput
//! ```
//!
//! The comparison is deliberately small: one JSON body, four enforcement policies, two payload
//! sizes. It answers the question "what does schema enforcement cost per request?" rather than
//! modeling a whole application.

use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, header};
use axum::routing::post;
use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;

#[derive(Clone, Default)]
pub struct State;

roundhouse::app_state!(State);

#[roundhouse::schema]
#[derive(Serialize, Deserialize)]
pub struct Payload {
    #[roundhouse(min_length = 1, max_length = 512)]
    pub name: String,
    #[roundhouse(min_items = 1, max_items = 64)]
    pub tags: Vec<String>,
}

mod routes {
    use super::*;

    routes! {
        "/validated" => { post(echo, operation_id = "bench_validated") },
        "/opaque" => { post(echo, operation_id = "bench_opaque", validate_body = false) },
        "/off" => { post(echo, operation_id = "bench_off", validate = false, unverified = "benchmark opt-out") },
    }

    #[handler]
    pub async fn echo(Json(payload): Json<Payload>) -> Json<Payload> {
        Json(payload)
    }
}

fn roundhouse_router() -> Router<()> {
    roundhouse::app!(State, routes)
        .openapi(|| roundhouse::openapi::Info::new("Throughput", "0.1.0"))
        .permissive()
        .build()
        .router()
}

fn axum_router() -> Router {
    Router::new().route(
        "/plain",
        post(|axum::Json(payload): axum::Json<Payload>| async move { Json(payload) }),
    )
}

fn request(path: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("a valid benchmark request")
}

fn bench(
    runtime: &tokio::runtime::Runtime,
    router: Router,
    path: &str,
    body: &'static str,
) -> Duration {
    // Warm up the route and the allocator before the measured loop.
    for _ in 0..32 {
        runtime
            .block_on(router.clone().oneshot(request(path, body)))
            .expect("the router answers");
    }

    let iterations = if body.len() > 16 * 1024 { 500 } else { 5_000 };
    let start = Instant::now();
    for _ in 0..iterations {
        runtime
            .block_on(router.clone().oneshot(request(path, body)))
            .expect("the router answers");
    }
    start.elapsed() / iterations
}

fn small_body() -> &'static str {
    r#"{"name":"roundhouse","tags":["rust","openapi"]}"#
}

fn large_body() -> &'static str {
    // ~64 KiB of payload, with the same schema.
    Box::leak(
        format!(
            r#"{{"name":"roundhouse","tags":[{}]}}"#,
            (0..2_000)
                .map(|index| format!("\"tag-{index}\""))
                .collect::<Vec<_>>()
                .join(",")
        )
        .into_boxed_str(),
    )
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");

    let validated = roundhouse_router();
    let opaque = roundhouse_router();
    let off = roundhouse_router();
    let plain = axum_router();

    for (label, body) in [("small", small_body()), ("large", large_body())] {
        println!("payload: {label} ({} bytes)", body.len());
        println!(
            "  plain axum           {:>8.2} µs",
            bench(&runtime, plain.clone(), "/plain", body).as_secs_f64() * 1e6
        );
        println!(
            "  roundhouse (schema)  {:>8.2} µs",
            bench(&runtime, validated.clone(), "/validated", body).as_secs_f64() * 1e6
        );
        println!(
            "  validate_body=false  {:>8.2} µs",
            bench(&runtime, opaque.clone(), "/opaque", body).as_secs_f64() * 1e6
        );
        println!(
            "  validate=false       {:>8.2} µs",
            bench(&runtime, off.clone(), "/off", body).as_secs_f64() * 1e6
        );
    }
}
