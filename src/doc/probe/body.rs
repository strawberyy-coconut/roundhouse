use std::any::TypeId;
use std::collections::BTreeMap;

use utoipa::openapi::schema::Components;

use super::super::{
    ApiSchema, BodyDoc, HandlerDocs, PartsDoc, PartsSchema,
    body::{json_body, json_body_to_schema},
};

/// A probe for a JSON body whose type has an `ApiSchema` (including composite bodies such as
/// `Vec<T>` that have no component of their own).
pub struct JsonBodyProbe<T>(std::marker::PhantomData<T>);

impl<T> JsonBodyProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for JsonBodyProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeJsonBody {
    fn probe_json_body(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: ApiSchema + 'static> ProbeJsonBody for &&JsonBodyProbe<axum::Json<T>> {
    fn probe_json_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(json_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
    }
}

impl<T: ApiSchema + 'static> ProbeJsonBody
    for &&JsonBodyProbe<crate::extract::Strict<axum::Json<T>>>
{
    fn probe_json_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        docs.body = Some(json_body::<T>(components));
        docs.body_type = Some(|| TypeId::of::<T>());
    }
}

impl<T> ProbeJsonBody for &&&JsonBodyProbe<T> {
    fn probe_json_body(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
}

/// A second body probe for types that derive only `ToSchema`.
///
/// Kept separate from `JsonBodyProbe` so the two autoref chains do not overlap: the `ApiSchema`
/// probe is exact at `&&`, this one is exact at `&&` for its own probe type, and each has its own
/// `&&&` no-op fallback. The handler calls both; only one sets `docs.body`.
pub struct ToSchemaBodyProbe<T>(std::marker::PhantomData<T>);

impl<T> ToSchemaBodyProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ToSchemaBodyProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeToSchemaBody {
    fn probe_to_schema_body(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: utoipa::ToSchema + 'static> ProbeToSchemaBody for &&ToSchemaBodyProbe<axum::Json<T>> {
    fn probe_to_schema_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        if docs.body.is_none() {
            docs.body = Some(json_body_to_schema::<T>(components));
            docs.body_type = Some(|| TypeId::of::<T>());
        }
    }
}

impl<T: utoipa::ToSchema + 'static> ProbeToSchemaBody
    for &&ToSchemaBodyProbe<crate::extract::Strict<axum::Json<T>>>
{
    fn probe_to_schema_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        if docs.body.is_none() {
            docs.body = Some(json_body_to_schema::<T>(components));
            docs.body_type = Some(|| TypeId::of::<T>());
        }
    }
}

impl<T> ProbeToSchemaBody for &&&ToSchemaBodyProbe<T> {
    fn probe_to_schema_body(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
}

/// A probe for the path/query types an operation uses.
///
/// The engine compiles the schema for each parts type once. The handler macro invokes this at the
/// concrete argument type, so a scalar `Path<T>` without a `PartsSchema` impl simply contributes
/// nothing.
pub struct PartsProbe<T>(std::marker::PhantomData<T>);

impl<T> PartsProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for PartsProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeParts {
    fn probe_parts(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: PartsSchema + 'static> ProbeParts for &&PartsProbe<axum::extract::Path<T>> {
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts for &&PartsProbe<crate::extract::RepeatedQuery<T>> {
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts for &&PartsProbe<axum::extract::Query<T>> {
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts
    for &&PartsProbe<crate::extract::Strict<axum::extract::Path<T>>>
{
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts
    for &&PartsProbe<crate::extract::Strict<axum::extract::Query<T>>>
{
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T: PartsSchema + 'static> ProbeParts
    for &&PartsProbe<crate::extract::Strict<crate::extract::RepeatedQuery<T>>>
{
    fn probe_parts(self, _components: &mut Components, docs: &mut HandlerDocs) {
        docs.parts.push(PartsDoc::new::<T>());
    }
}

impl<T> ProbeParts for &&&PartsProbe<T> {
    fn probe_parts(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
}

/// A second, narrower probe for a foreign JSON body that has a utoipa schema.
///
/// The `ToSchema` bound must be resolved at the handler's concrete argument type, not inside the
/// generic extractor probe, so this is a separate autoref chain: `&&ForeignBodyProbe<axum::Json<T>>`
/// applies when `T: ToSchema`, and the `&&&` implementation for everything else does nothing.
pub struct ForeignBodyProbe<T>(std::marker::PhantomData<T>);

impl<T> ForeignBodyProbe<T> {
    pub const fn new() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T> Default for ForeignBodyProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait ProbeForeignBody {
    fn probe_foreign_body(self, components: &mut Components, docs: &mut HandlerDocs);
}

impl<T: utoipa::ToSchema> ProbeForeignBody for &&ForeignBodyProbe<axum::Json<T>> {
    fn probe_foreign_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        use utoipa::ToSchema;
        let name = T::name().into_owned();
        // `ApiSchema::register` already inserted this type (with its schema patches, such as
        // `url` → `uri` and `uniqueItems`). Do not overwrite it with the raw `ToSchema` output.
        if !components.schemas.contains_key(&name) {
            let mut referenced = Vec::new();
            <T as ToSchema>::schemas(&mut referenced);
            for (component, schema) in referenced {
                components.schemas.insert(component, schema);
            }
            components.schemas.insert(name.clone(), T::schema());
        }
        if docs.body.is_none() {
            docs.body = Some(BodyDoc {
                content_type: "application/json".to_string(),
                schema: utoipa::openapi::Ref::from_schema_name(name).into(),
                encoding: BTreeMap::new(),
                required: true,
                description: None,
            });
        }
    }
}

impl<T: utoipa::ToSchema> ProbeForeignBody
    for &&ForeignBodyProbe<crate::extract::Strict<axum::Json<T>>>
{
    fn probe_foreign_body(self, components: &mut Components, docs: &mut HandlerDocs) {
        (&&ForeignBodyProbe::<axum::Json<T>>::new()).probe_foreign_body(components, docs);
    }
}

impl<T> ProbeForeignBody for &&&ForeignBodyProbe<T> {
    fn probe_foreign_body(self, _components: &mut Components, _docs: &mut HandlerDocs) {}
}

// axum's other extractors are known and contribute nothing the document cannot already see.
