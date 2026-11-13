use roundhouse::prelude::*;

struct Scheme;

impl AuthScheme for Scheme {
    type State = ();
    type Identity = ();

    const NAME: &'static str = "scheme";
    const KIND: roundhouse::SecuritySchemeKind = roundhouse::SecuritySchemeKind::HttpBearer;
}

struct Read;

impl Scopes<Scheme> for Read {
    const NAMES: &'static [&'static str] = &["read"];
}

fn main() {}
