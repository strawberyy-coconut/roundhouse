//! The fallible build must be total: wiring mistakes are `BuildError`s, not panics.

use roundhouse::prelude::*;
use roundhouse::{App, BuildError};

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
fn a_route_wired_to_the_wrong_state_is_an_error() {
    let error = App::new(AppState)
        .scope(concat!(module_path!(), "::wrong"))
        .try_build()
        .err()
        .expect("a state mismatch must not build");
    assert!(matches!(error, BuildError::StateMismatch { .. }));
    assert!(error.to_string().contains("needs"));
}

#[test]
fn a_scope_that_matches_nothing_is_an_error() {
    let error = App::new(AppState)
        .scope("no_such_module")
        .try_build()
        .err()
        .expect("an empty scope must not build");
    assert!(matches!(error, BuildError::EmptyScope { .. }));
}

#[test]
fn try_build_never_panics_on_negative_fixtures() {
    // `catch_unwind` is the guard the review asks for: every fixture above returns a value.
    for scope in [
        concat!(module_path!(), "::wrong"),
        "no_such_module",
        module_path!(),
    ] {
        let outcome = std::panic::catch_unwind(|| {
            App::new(AppState)
                .scope(scope)
                .try_build()
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        assert!(outcome.is_ok(), "`try_build` panicked for scope `{scope}`");
    }
}
