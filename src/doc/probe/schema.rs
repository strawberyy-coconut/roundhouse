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

use super::super::{
    ApiSchema, ResponseDoc, SchemaContext, register_self,
    response_doc::{json_response_doc, json_response_doc_to_schema},
};

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
        value.validate_at(pointer, errors);
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

/// A concrete-type probe for a typed `response(status, Type)` declaration.
pub struct JsonResponseDocProbe<T>(std::marker::PhantomData<T>);

impl<T> JsonResponseDocProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for JsonResponseDocProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeJsonResponseDoc {
    fn probe_json_response_doc(self, context: &mut dyn SchemaContext, status: u16) -> ResponseDoc;
}

impl<T: ApiSchema> ProbeJsonResponseDoc for &JsonResponseDocProbe<T> {
    fn probe_json_response_doc(self, context: &mut dyn SchemaContext, status: u16) -> ResponseDoc {
        json_response_doc::<T>(context, status)
    }
}

impl<T: utoipa::ToSchema> ProbeJsonResponseDoc for &&JsonResponseDocProbe<T> {
    fn probe_json_response_doc(self, context: &mut dyn SchemaContext, status: u16) -> ResponseDoc {
        json_response_doc_to_schema::<T>(context, status)
    }
}
