use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

mod one {
    use super::*;
    mount!("/dup");
    routes! { get("/x", x) }
    #[handler]
    pub async fn x() -> &'static str {
        "one"
    }
}

mod two {
    use super::*;
    mount!("/dup");
    routes! { get("/x", x) }
    #[handler]
    pub async fn x() -> &'static str {
        "two"
    }
}

#[test]
#[should_panic(expected = "duplicate route")]
fn duplicate_routes_panic_with_both_locations() {
    roundhouse::App::new(State).scope(module_path!()).build();
}
