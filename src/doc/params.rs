use super::{Param, Params, SchemaContext};

pub fn path_params<T: Params>(context: &mut dyn SchemaContext) -> Vec<Param> {
    T::params(context)
}

pub fn stored_query_params_for<T: Params + ?Sized>(context: &mut dyn SchemaContext) -> Vec<Param> {
    <T as Params>::params(context)
}
