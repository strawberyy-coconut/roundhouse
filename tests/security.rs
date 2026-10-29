//! A `security` declaration is a claim about the router, not an annotation.
//!
//! These fixtures deliberately declare security without an enforcer. The build must reject them
//! instead of publishing a scheme nothing checks.

use roundhouse::prelude::*;
use roundhouse::{App, BuildError};

#[derive(Clone, Default)]
pub struct State;

mod unenforced {
    use super::*;

    mount!("/unenforced", security = ["bearer"]);

    routes! { get("/x", x) }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

mod invented {
    use super::*;

    mount!("/invented", security = ["oauth"]);

    routes! { get("/x", x) }

    #[handler]
    pub async fn x() -> &'static str {
        "x"
    }
}

fn scoped(module: &'static str) -> roundhouse::App<State> {
    App::new(State)
        .scope(module)
        .openapi(|| roundhouse::openapi::Info::new("Security", "0.1.0"))
}

#[test]
fn mount_security_without_an_enforcer_is_a_build_error() {
    let error = scoped(concat!(module_path!(), "::unenforced"))
        .try_build()
        .err()
        .expect("security without an enforcer must not build");
    assert!(matches!(
        error,
        BuildError::UnverifiedSecurity { ref name, .. } if name == "bearer"
    ));
}

#[test]
fn an_unknown_scheme_is_not_guessed_from_its_name() {
    let error = scoped(concat!(module_path!(), "::invented"))
        .try_build()
        .err()
        .expect("an unregistered scheme must not fabricate a kind");
    assert!(matches!(
        error,
        BuildError::UnverifiedSecurity { ref name, .. } if name == "oauth"
    ));
    assert!(!error.to_string().contains("apiKey"));
}
