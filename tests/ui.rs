#[test]
fn removed_attributes_are_rejected() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
