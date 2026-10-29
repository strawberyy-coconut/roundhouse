//! The stable facade over utoipa's document model.
//!
//! Applications configure an app with [`Info`] and read a [`OpenApi`] document. Both are
//! roundhouse types; `as_utoipa`/`into_utoipa` are the explicit escape hatch for code that needs
//! utoipa itself. The rest of the document model (schemas, paths, responses) is re-exported or
//! accepted through utoipa-compatible values by the derives.
//!
//! [`Components`] is the component table passed to low-level `ApiSchema`/`ApiResponse`
//! implementations. It is currently the utoipa table so hand-written schema code stays
//! source-compatible; keeping the name here gives a single place to change when utoipa moves.

use serde::{Deserialize, Serialize};

/// Components table passed to hand-written schema and response documentation.
pub use utoipa::openapi::schema::Components;

/// API metadata, the `info` object of an OpenAPI document.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Info(utoipa::openapi::Info);

impl Info {
    /// Construct `Info` from a title and version.
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        let mut info = utoipa::openapi::Info::default();
        info.title = title.into();
        info.version = version.into();
        Self(info)
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.0.title = title.into();
        self
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.0.version = version.into();
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.0.description = Some(description.into());
        self
    }

    pub fn terms_of_service(mut self, terms: impl Into<String>) -> Self {
        self.0.terms_of_service = Some(terms.into());
        self
    }

    /// Borrow the underlying utoipa `Info`, for interop with utoipa's builders.
    pub fn as_utoipa(&self) -> &utoipa::openapi::Info {
        &self.0
    }

    /// Consume this value and return the underlying utoipa `Info`.
    pub fn into_utoipa(self) -> utoipa::openapi::Info {
        self.0
    }
}

impl From<utoipa::openapi::Info> for Info {
    fn from(info: utoipa::openapi::Info) -> Self {
        Self(info)
    }
}

impl From<Info> for utoipa::openapi::Info {
    fn from(info: Info) -> Self {
        info.0
    }
}

impl std::ops::Deref for Info {
    type Target = utoipa::openapi::Info;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Info {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Serialize for Info {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Info {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        utoipa::openapi::Info::deserialize(deserializer).map(Self)
    }
}

/// A generated OpenAPI document.
#[derive(Clone, Default, PartialEq)]
pub struct OpenApi(utoipa::openapi::OpenApi);

impl OpenApi {
    /// Borrow the underlying utoipa document.
    pub fn as_utoipa(&self) -> &utoipa::openapi::OpenApi {
        &self.0
    }

    /// Mutably borrow the underlying utoipa document.
    pub fn as_utoipa_mut(&mut self) -> &mut utoipa::openapi::OpenApi {
        &mut self.0
    }

    /// Consume this value and return the underlying utoipa document.
    pub fn into_utoipa(self) -> utoipa::openapi::OpenApi {
        self.0
    }

    /// Serialize the document as pretty JSON.
    pub fn to_pretty_json(&self) -> Result<String, serde_json::Error> {
        self.0.to_pretty_json()
    }

    /// Serialize the document as compact JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        self.0.to_json()
    }
}

impl From<utoipa::openapi::OpenApi> for OpenApi {
    fn from(document: utoipa::openapi::OpenApi) -> Self {
        Self(document)
    }
}

impl From<OpenApi> for utoipa::openapi::OpenApi {
    fn from(document: OpenApi) -> Self {
        document.0
    }
}

impl std::ops::Deref for OpenApi {
    type Target = utoipa::openapi::OpenApi;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for OpenApi {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Serialize for OpenApi {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OpenApi {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        utoipa::openapi::OpenApi::deserialize(deserializer).map(Self)
    }
}
