//! The framework's validation engine is `jsonschema` and nothing else.
//!
//! These assertions are deliberately crude: adding a `validator`, `garde`, `axum-valid` or
//! `validify` dependency should be a reviewable change, not an accidental one.

fn declares(manifest: &str, crate_name: &str) -> bool {
    manifest.lines().any(|line| {
        let line = line.trim_start();
        line.strip_prefix(crate_name)
            .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('='))
    })
}

#[test]
fn no_external_validation_crate_is_a_dependency() {
    let manifests = [
        ("roundhouse", include_str!("../Cargo.toml")),
        (
            "roundhouse-schema",
            include_str!("../roundhouse-schema/Cargo.toml"),
        ),
        (
            "roundhouse-cli",
            include_str!("../roundhouse-cli/Cargo.toml"),
        ),
    ];
    for (name, manifest) in manifests {
        for forbidden in ["validator", "garde", "axum-valid", "validify"] {
            assert!(
                !declares(manifest, forbidden),
                "{name} must not depend on `{forbidden}`"
            );
        }
    }
}

#[test]
fn jsonschema_is_the_runtime_engine() {
    let manifest = include_str!("../roundhouse-schema/Cargo.toml");
    assert!(declares(manifest, "jsonschema"));
}
