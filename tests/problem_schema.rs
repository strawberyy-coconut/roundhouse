//! The documented Problem schema and the wire body share one definition.

#[test]
fn documented_problem_schema_matches_the_wire_schema() {
    let documented = serde_json::to_value(roundhouse::doc::problem_schema())
        .expect("the utoipa schema serializes");
    let wire = roundhouse::problem::problem_schema();
    assert_eq!(documented, wire);
}
