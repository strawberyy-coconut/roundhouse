use roundhouse::prelude::*;

struct NotAScheme;

mount!(security = [NotAScheme]);

fn main() {}
