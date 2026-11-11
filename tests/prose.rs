//! `prose = include_str!(..)` supplies an operation summary and description from one file.

use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct State;

const INLINE: &str = "An inline summary.\n\nAn inline description.";

mod from_file {
    use super::*;

    routes! {
        "/" => {
            get(show, prose = include_str!("fixtures/show.md")),
        },
    }

    #[handler]
    pub async fn show() -> &'static str {
        "thing"
    }
}

mod from_const {
    use super::*;

    routes! {
        "/" => {
            get(show, prose = INLINE),
        },
    }

    #[handler]
    pub async fn show() -> &'static str {
        "thing"
    }
}

fn document(module: &'static roundhouse::Module) -> serde_json::Value {
    let built = roundhouse::App::new(State)
        .include(module)
        .openapi(|| roundhouse::openapi::Info::new("Prose", "0.1.0"))
        .build();
    serde_json::to_value(built.document()).unwrap()
}

#[test]
fn include_str_supplies_summary_and_description() {
    let document = document(&from_file::MODULE);
    let operation = &document["paths"]["/"]["get"];
    assert_eq!(operation["summary"], "Read one thing.");
    assert_eq!(
        operation["description"],
        "Returns the stored representation, or 404 if it does not exist."
    );
}

#[test]
fn a_const_string_supplies_summary_and_description() {
    let document = document(&from_const::MODULE);
    let operation = &document["paths"]["/"]["get"];
    assert_eq!(operation["summary"], "An inline summary.");
    assert_eq!(operation["description"], "An inline description.");
}
