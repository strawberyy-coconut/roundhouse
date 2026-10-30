use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, Schema};

use super::super::{
    ApiSchema, ResponseDoc, register_self,
    response_doc::{
        json_response, json_response_doc, json_response_doc_to_schema, json_response_to_schema,
        text_response, text_response_to_schema,
    },
};

pub struct SchemaFor<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> SchemaFor<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for SchemaFor<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeSchemaFor {
    fn probe_schema(self) -> RefOr<Schema>;
    fn probe_register(self, components: &mut Components);
}

impl<T: ApiSchema + ?Sized> ProbeSchemaFor for &SchemaFor<T> {
    fn probe_schema(self) -> RefOr<Schema> {
        T::schema()
    }

    fn probe_register(self, components: &mut Components) {
        T::register(components);
    }
}

impl<T: utoipa::ToSchema + ?Sized> ProbeSchemaFor for &&SchemaFor<T> {
    fn probe_schema(self) -> RefOr<Schema> {
        <T as utoipa::PartialSchema>::schema()
    }

    fn probe_register(self, components: &mut Components) {
        register_self::<T>(components);
    }
}

/// A concrete-type probe for recursive `validate_with` traversal.
///
/// The specific impl calls the field type's [`ApiSchema::validate_at`]; the fallback is a no-op
/// for a plain `ToSchema` field. Call this at the concrete field type.
pub struct ValidateFor<T: ?Sized>(std::marker::PhantomData<T>);

impl<T: ?Sized> ValidateFor<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ?Sized> Default for ValidateFor<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeValidateFor<T: ?Sized> {
    fn probe_validate_for(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    );
}

impl<T: ApiSchema + ?Sized> ProbeValidateFor<T> for &ValidateFor<T> {
    fn probe_validate_for(
        self,
        value: &T,
        pointer: &str,
        errors: &mut crate::validate::ValidationErrors,
    ) {
        value.validate_at(pointer, errors);
    }
}

impl<T: utoipa::ToSchema + ?Sized> ProbeValidateFor<T> for &&ValidateFor<T> {
    fn probe_validate_for(
        self,
        _value: &T,
        _pointer: &str,
        _errors: &mut crate::validate::ValidationErrors,
    ) {
    }
}

/// A concrete-type probe for a JSON response body.
pub struct JsonResponseProbe<T>(std::marker::PhantomData<T>);

impl<T> JsonResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for JsonResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeJsonResponse {
    fn probe_json_response(self, components: &mut Components, status: u16) -> Vec<ResponseDoc>;
}

impl<T: ApiSchema> ProbeJsonResponse for &JsonResponseProbe<T> {
    fn probe_json_response(self, components: &mut Components, status: u16) -> Vec<ResponseDoc> {
        json_response::<T>(components, status)
    }
}

impl<T> ProbeJsonResponse for &&JsonResponseProbe<T> {
    fn probe_json_response(self, _components: &mut Components, _status: u16) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

/// A second response probe for types that derive only `ToSchema`.
pub struct ToSchemaJsonResponseProbe<T>(std::marker::PhantomData<T>);

impl<T> ToSchemaJsonResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ToSchemaJsonResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeToSchemaJsonResponse {
    fn probe_to_schema_json_response(
        self,
        components: &mut Components,
        status: u16,
    ) -> Vec<ResponseDoc>;
}

impl<T: utoipa::ToSchema> ProbeToSchemaJsonResponse for &ToSchemaJsonResponseProbe<T> {
    fn probe_to_schema_json_response(
        self,
        components: &mut Components,
        status: u16,
    ) -> Vec<ResponseDoc> {
        json_response_to_schema::<T>(components, status)
    }
}

impl<T> ProbeToSchemaJsonResponse for &&ToSchemaJsonResponseProbe<T> {
    fn probe_to_schema_json_response(
        self,
        _components: &mut Components,
        _status: u16,
    ) -> Vec<ResponseDoc> {
        Vec::new()
    }
}

/// A concrete-type probe for a `responses(status = Type)` declaration.
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
    fn probe_json_response_doc(self, components: &mut Components, status: u16) -> ResponseDoc;
}

impl<T: ApiSchema> ProbeJsonResponseDoc for &JsonResponseDocProbe<T> {
    fn probe_json_response_doc(self, components: &mut Components, status: u16) -> ResponseDoc {
        json_response_doc::<T>(components, status)
    }
}

impl<T: utoipa::ToSchema> ProbeJsonResponseDoc for &&JsonResponseDocProbe<T> {
    fn probe_json_response_doc(self, components: &mut Components, status: u16) -> ResponseDoc {
        json_response_doc_to_schema::<T>(components, status)
    }
}

/// A concrete-type probe for a non-JSON response body.
pub struct TextResponseProbe<T>(std::marker::PhantomData<T>);

impl<T> TextResponseProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for TextResponseProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeTextResponse {
    fn probe_text_response(
        self,
        components: &mut Components,
        status: u16,
        content_type: &str,
    ) -> ResponseDoc;
}

impl<T: ApiSchema> ProbeTextResponse for &TextResponseProbe<T> {
    fn probe_text_response(
        self,
        components: &mut Components,
        status: u16,
        content_type: &str,
    ) -> ResponseDoc {
        text_response::<T>(components, status, content_type)
    }
}

impl<T: utoipa::ToSchema> ProbeTextResponse for &&TextResponseProbe<T> {
    fn probe_text_response(
        self,
        components: &mut Components,
        status: u16,
        content_type: &str,
    ) -> ResponseDoc {
        text_response_to_schema::<T>(components, status, content_type)
    }
}
