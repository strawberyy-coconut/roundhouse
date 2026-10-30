use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

mod one {
    use super::*;
    routes! { prefix = "/dup", get("/x", x) }
    #[handler]
    pub async fn x() -> &'static str {
        "one"
    }
}

mod two {
    use super::*;
    routes! { prefix = "/dup", get("/x", x) }
    #[handler]
    pub async fn x() -> &'static str {
        "two"
    }
}

#[test]
#[should_panic(expected = "duplicate route")]
fn duplicate_routes_panic_with_both_locations() {
    roundhouse::App::new(State)
        .include(&one::MODULE)
        .include(&two::MODULE)
        .build();
}
