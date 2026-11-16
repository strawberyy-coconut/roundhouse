use super::{Param, ParamsDoc, SchemaContext};

pub fn path_params<T: ParamsDoc>(context: &mut dyn SchemaContext) -> Vec<Param> {
    T::params(context)
}

pub fn stored_query_params_for<T: ParamsDoc + ?Sized>(
    context: &mut dyn SchemaContext,
) -> Vec<Param> {
    <T as ParamsDoc>::params(context)
}
