use roundhouse::prelude::*;

pub mod health;
pub mod notes;

// The route table is the module tree. Handlers live in child modules; the table names the
// composed paths, tags and policy once.
routes! {
    tags = ["api"],

    "/healthz" => {
        get(health::healthz, tags = ["ops"]),
    },

    "/notes" => {
        get(notes::list, tags = ["notes"]),
        post(notes::create, tags = ["notes"]),
    },

    "/notes/{id}" => {
        get(notes::show, tags = ["notes"]),
    },
}
