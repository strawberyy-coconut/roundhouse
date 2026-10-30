//! The fallible build must be total: wiring mistakes are `BuildError`s, not panics.

use roundhouse::prelude::*;
use roundhouse::{App, BuildError};

#[derive(Clone, Default)]
pub struct AppState;

#[derive(Clone, Default)]
pub struct OtherState;

mod wrong {
    use super::*;

    routes! { prefix = "/wrong", get("/x", x) }

    #[handler]
    pub async fn x(State(_): State<OtherState>) -> &'static str {
        "wrong"
    }
}

#[test]
fn a_route_wired_to_the_wrong_state_is_an_error() {
    let error = App::new(AppState)
        .include(&wrong::MODULE)
        .try_build()
        .err()
        .expect("a state mismatch must not build");
    assert!(matches!(error, BuildError::StateMismatch { .. }));
    assert!(error.to_string().contains("needs"));
}

#[test]
fn an_app_with_no_fragment_is_an_error() {
    let error = App::new(AppState)
        .try_build()
        .err()
        .expect("an empty app must not build");
    assert!(matches!(error, BuildError::EmptyApp));
}

#[test]
fn try_build_never_panics_on_negative_fixtures() {
    // `catch_unwind` is the guard the review asks for: every fixture above returns a value.
    for fixture in 0..2 {
        let outcome = std::panic::catch_unwind(|| {
            let app = App::new(AppState);
            let app = if fixture == 0 {
                app.include(&wrong::MODULE)
            } else {
                app
            };
            app.try_build()
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        assert!(
            outcome.is_ok(),
            "`try_build` panicked for fixture {fixture}"
        );
    }
}
