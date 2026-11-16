//! Startup cost: building the contract in-process versus consuming a committed bundle.
//!
//! Run with:
//!
//! ```sh
//! cargo bench -p roundhouse --bench build
//! ```
//!
//! The first number is the full `contract` build (`App::build` with no bundle): schema
//! derivation, the document builder and route resolution. The second is the production
//! `contract-runtime` path: route resolution plus contract compilation. Both are measured on the
//! same tiny route table, so the difference is the documentation work 0.4 moved off the runtime.

use std::time::Instant;

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

roundhouse::app_state!(State);

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Message {
    pub text: String,
}

mod routes {
    use super::*;

    routes! {
        "/hello" => { get(hello) },
    }

    #[handler]
    pub async fn hello() -> Json<Message> {
        Json(Message {
            text: "hi".to_string(),
        })
    }
}

fn app() -> roundhouse::App<State> {
    roundhouse::App::new(State)
        .fragment(routes::__roundhouse_fragment())
        .openapi(|| roundhouse::openapi::Info::new("Build", "0.1.0"))
        .permissive()
}

fn main() {
    const ITERATIONS: u32 = 20;

    // Warm both paths so allocator and lazy-static costs land outside the measurement.
    let _ = app().try_build().expect("contract mode builds");
    let contract =
        roundhouse::Contract::<State>::new(roundhouse::openapi::Info::new("Build", "0.1.0"))
            .fragment(routes::__roundhouse_fragment())
            .permissive()
            .build()
            .expect("contract builds");
    let bundle = roundhouse::ContractBundle::new(
        Box::leak(
            contract
                .contract_json()
                .expect("contract serializes")
                .into_boxed_str(),
        ),
        Box::leak(contract.openapi_json().into_boxed_str()),
    );
    let _ = app()
        .contract(bundle)
        .try_build()
        .expect("bundle mode builds");

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = app().try_build().expect("contract mode builds");
    }
    let contract_build = start.elapsed() / ITERATIONS;

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = app()
            .contract(bundle)
            .try_build()
            .expect("bundle mode builds");
    }
    let bundle_startup = start.elapsed() / ITERATIONS;

    println!("contract build (App::build, no bundle): {contract_build:?}");
    println!("committed bundle startup (contract-runtime): {bundle_startup:?}");
    println!(
        "documentation overhead: {:.2}x",
        contract_build.as_secs_f64() / bundle_startup.as_secs_f64()
    );
}
