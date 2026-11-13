use std::collections::BTreeMap;

#[cfg(feature = "multipart")]
use super::ResponseDoc;
use super::{ApiSchema, BodyDoc, SchemaContext, schema};

/// Register the schemas and build the response for a JSON body.
pub fn json_body<T: ApiSchema>(context: &mut dyn SchemaContext) -> BodyDoc {
    T::register(context);
    BodyDoc {
        content_type: "application/json".to_string(),
        schema: T::schema(),
        encoding: BTreeMap::new(),
        required: true,
        description: None,
    }
}

/// Register the components referenced by a plain `ToSchema` type.
pub fn register_to_schema<T: utoipa::ToSchema + ?Sized>(context: &mut dyn SchemaContext) {
    let mut referenced = Vec::new();
    <T as utoipa::ToSchema>::schemas(&mut referenced);
    let type_name = std::any::type_name::<T>();
    for (name, schema) in referenced {
        // `schemas()` has no Rust type identity for each entry. Identical shapes are the same
        // shared type; a different shape under the same name is a real collision.
        let collision = context
            .components()
            .schemas
            .get(&name)
            .is_some_and(|existing| existing != &schema);
        if collision {
            context.record_collision(&name, "<registered schema>", type_name);
            continue;
        }
        if context.components().schemas.contains_key(&name) {
            continue;
        }
        context.components().schemas.insert(name, schema);
    }
}

/// Register just this type as a component, without walking its `schemas()`.
///
/// `ToSchema::schemas()` recurses for self-referential types; registration driven by an enclosing
/// `ApiSchema` derive calls this per field instead, so a recursive field cannot overflow.
pub fn register_self<T: utoipa::ToSchema + ?Sized>(context: &mut dyn SchemaContext) {
    let name = <T as utoipa::ToSchema>::name().into_owned();
    // Composite types have no stable component identity (`Vec`, `Option`, ...). Their item
    // schemas are inlined or registered by the concrete item type; never invent a component.
    if matches!(
        name.as_str(),
        "Vec"
            | "Option"
            | "Box"
            | "Arc"
            | "Rc"
            | "Cow"
            | "HashMap"
            | "BTreeMap"
            | "HashSet"
            | "BTreeSet"
            | "LinkedList"
            | "IndexMap"
            | "IndexSet"
            | "String"
            | "str"
            | "Value"
    ) {
        return;
    }
    match context.claim(&name, std::any::type_name::<T>()) {
        roundhouse_schema::Claim::Already | roundhouse_schema::Claim::Collision => {
            return;
        }
        roundhouse_schema::Claim::Claimed | roundhouse_schema::Claim::Inactive => {}
    }
    let components = context.components();
    if components.schemas.contains_key(&name) {
        return;
    }
    components
        .schemas
        .insert(name, <T as utoipa::PartialSchema>::schema());
}

/// A JSON body documented from `ToSchema` alone, without roundhouse hooks.
pub fn json_body_to_schema<T: utoipa::ToSchema>(context: &mut dyn SchemaContext) -> BodyDoc {
    register_to_schema::<T>(context);
    BodyDoc {
        content_type: "application/json".to_string(),
        schema: <T as utoipa::PartialSchema>::schema(),
        encoding: BTreeMap::new(),
        required: true,
        description: None,
    }
}

#[cfg(feature = "multipart")]
pub fn multipart_body<F: crate::multipart::MultipartForm>(
    context: &mut dyn SchemaContext,
) -> BodyDoc {
    let encoding = F::fields()
        .iter()
        .filter(|field| field.kind.is_file() && !field.accept.is_empty())
        .map(|field| {
            let content_type = field.accept.join(", ");
            (
                field.name.to_string(),
                utoipa::openapi::encoding::EncodingBuilder::new()
                    .content_type(Some(content_type))
                    .build(),
            )
        })
        .collect();
    BodyDoc {
        content_type: "multipart/form-data".to_string(),
        schema: F::schema(context),
        encoding,
        required: true,
        description: None,
    }
}

#[cfg(feature = "multipart")]
pub fn multipart_failures() -> Vec<ResponseDoc> {
    vec![
        ResponseDoc::problem(400, "Bad Request"),
        ResponseDoc::problem(413, "Payload Too Large"),
        ResponseDoc::problem(415, "Unsupported Media Type"),
    ]
}

pub fn form_body<T: ApiSchema>(context: &mut dyn SchemaContext) -> BodyDoc {
    T::register(context);
    BodyDoc {
        content_type: "application/x-www-form-urlencoded".to_string(),
        schema: T::schema(),
        encoding: BTreeMap::new(),
        required: true,
        description: None,
    }
}

/// A raw `Bytes` or `Body` request. The content type is the contract, not a schema.
pub fn binary_body() -> BodyDoc {
    BodyDoc {
        content_type: "application/octet-stream".to_string(),
        schema: schema::string_with_format(utoipa::openapi::schema::SchemaFormat::KnownFormat(
            utoipa::openapi::schema::KnownFormat::Binary,
        )),
        encoding: BTreeMap::new(),
        required: true,
        description: None,
    }
}
