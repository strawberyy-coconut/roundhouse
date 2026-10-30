use roundhouse::prelude::*;

struct NotAScheme;

routes! { security = [NotAScheme] }

fn main() {}
