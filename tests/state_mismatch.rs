use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct AppState;

#[derive(Clone, Default)]
pub struct OtherState;

mod wrong {
    use super::*;
    mount!("/wrong");
    routes! { get("/x", x) }
    #[handler]
    pub async fn x(State(_): State<OtherState>) -> &'static str {
        "wrong"
    }
}

#[test]
#[should_panic(expected = "app was built with")]
fn a_route_wired_to_the_wrong_state_fails_at_startup() {
    roundhouse::App::new(AppState).scope(module_path!()).build();
}
