//! A proc macro cannot evaluate `#[cfg]`, so entries gated by one are left to the resolved route
//! table. Two mutually exclusive declarations must not fail table-local duplicate checks.

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

roundhouse::app_state!(State);

mod mutually_exclusive {
    use super::*;

    routes! {
        "/x" => {
            #[cfg(any())]
            get(a, operation_id = "cfg_a"),
            #[cfg(any())]
            get(b, operation_id = "cfg_b"),
        },
    }

    #[handler]
    pub async fn a() -> &'static str {
        "a"
    }

    #[handler]
    pub async fn b() -> &'static str {
        "b"
    }
}

#[test]
fn a_cfg_gated_table_expands_without_a_false_duplicate() {}
