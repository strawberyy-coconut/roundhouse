use roundhouse::prelude::*;

use crate::schemes::BearerAuth;

pub mod protected;

routes! {
    tags = ["auth"],

    "/public" => {
        get(protected::public),
    },

    "/private" => {
        get(protected::private),
    },

    "/scoped" => {
        get(protected::scoped),
    },

    "/write" => {
        get(protected::write),
    },

    "/optional" => {
        get(protected::optional),
    },

    "/admin" => {
        get(protected::admin),
    },

    "/projects/{project_id}" => {
        get(protected::project),
    },

    prefix "/guarded" [security = [BearerAuth]] {
        "/whoami" => {
            get(protected::whoami),
        },
    },
}
