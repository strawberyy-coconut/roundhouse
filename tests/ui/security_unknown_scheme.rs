use roundhouse::prelude::*;

roundhouse::app_state!(());

struct NotAScheme;

routes! { security = [NotAScheme] }

fn main() {}
