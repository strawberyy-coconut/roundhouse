//! An app that resolved no routes must not start and 404 everything.

use roundhouse::{App, BuildError};

#[derive(Clone, Default)]
pub struct State;

#[test]
fn an_app_with_no_routes_is_an_error() {
    let error = App::new(State)
        .try_build()
        .err()
        .expect("an empty app must not build");
    assert!(matches!(error, BuildError::EmptyApp));
    assert!(error.to_string().contains("resolved no routes"));
}
