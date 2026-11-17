//! Concrete-type probes for schema and response documentation.
//!
//! The handler macro sees argument and return types as tokens. These probes use autoref
//! specialization to ask the concrete type what it can document:
//!
//! - `&SchemaProbe<T>` — `T: ApiSchema`;
//! - `&&SchemaProbe<T>` — `T: ToSchema` only;
//! - `&&&SchemaProbe<T>` — neither.
//!
//! Each level implements the same [`ProbeSchema`] methods, so one struct serves the schema,
//! description and `validate_with` questions. Probes are macro plumbing and live behind
//! `roundhouse::__private`.

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::Schema;

use super::super::{ApiSchema, SchemaContext, register_self};

pub struct SchemaProbe<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> SchemaProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for SchemaProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeSchema<T: ?Sized> {
    fn probe_schema(self) -> RefOr<Schema>;
    fn probe_description(self) -> Option<&'static str>;
    fn probe_register(self, context: &mut dyn SchemaContext);
    fn probe_validate_at(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    );
}

impl<T: ApiSchema + ?Sized> ProbeSchema<T> for &SchemaProbe<T> {
    fn probe_schema(self) -> RefOr<Schema> {
        T::schema()
    }

    fn probe_description(self) -> Option<&'static str> {
        T::description()
    }

    fn probe_register(self, context: &mut dyn SchemaContext) {
        T::register(context);
    }

    fn probe_validate_at(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    ) {
        use crate::probe::ProbeHook as _;
        (&crate::probe::HookProbe::<T>::new()).probe_validate_at(value, pointer, errors);
    }
}

impl<T: utoipa::ToSchema + ?Sized> ProbeSchema<T> for &&SchemaProbe<T> {
    fn probe_schema(self) -> RefOr<Schema> {
        <T as utoipa::PartialSchema>::schema()
    }

    fn probe_description(self) -> Option<&'static str> {
        None
    }

    fn probe_register(self, context: &mut dyn SchemaContext) {
        register_self::<T>(context);
    }

    fn probe_validate_at(
        self,
        _value: &T,
        _pointer: &str,
        _errors: &mut crate::validate::ValidationErrors,
    ) {
    }
}

impl<T: ?Sized> ProbeSchema<T> for &&&SchemaProbe<T> {
    fn probe_schema(self) -> RefOr<Schema> {
        crate::doc::schema::any_value()
    }

    fn probe_description(self) -> Option<&'static str> {
        None
    }

    fn probe_register(self, _context: &mut dyn SchemaContext) {}

    fn probe_validate_at(
        self,
        _value: &T,
        _pointer: &str,
        _errors: &mut crate::validate::ValidationErrors,
    ) {
    }
}
