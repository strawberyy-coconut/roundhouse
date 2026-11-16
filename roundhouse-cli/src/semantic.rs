//! Local semantic index for response inference.
//!
//! The analyzer has no compiler session, so this module resolves the parts of Rust that matter for
//! responses: helper and method return types, struct fields, type aliases, `From` impls and
//! `IntoResponse` implementations. It is deliberately conservative: an unresolved type is `None`,
//! never a guess.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use syn::{
    Expr, Fields, ImplItem, Item, ItemConst, ItemEnum, ItemFn, ItemImpl, ItemStatic, ItemStruct,
    ItemType, Pat, ReturnType, Type,
};

/// A syntactically resolved type: its outer name and any type arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TypeRef {
    pub name: String,
    pub args: Vec<TypeRef>,
}

impl TypeRef {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            args: Vec::new(),
        }
    }

    pub(crate) fn with_args(name: impl Into<String>, args: Vec<TypeRef>) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }

    pub(crate) fn is(&self, name: &str) -> bool {
        self.name == name
    }

    pub(crate) fn arg(&self, index: usize) -> Option<&TypeRef> {
        self.args.get(index)
    }

    /// The component name a JSON body would register for this type.
    ///
    /// Collections and wrappers are not assigned a component: their OpenAPI shape is an array or
    /// an object that the analyzer does not synthesize, so the status is still documented but
    /// the body stays unverified.
    pub(crate) fn schema_name(&self) -> Option<String> {
        match self.name.as_str() {
            "Json" | "Created" => self.arg(0)?.direct_schema_name(),
            "Vec" | "Option" | "Result" => None,
            _ => self.direct_schema_name(),
        }
    }

    fn direct_schema_name(&self) -> Option<String> {
        match self.name.as_str() {
            "Response" | "StatusCode" | "Body" | "Bytes" | "String" | "str" | "Unknown" => None,
            name if name.chars().next().is_some_and(char::is_uppercase) => Some(self.name.clone()),
            _ => None,
        }
    }
}

impl std::fmt::Display for TypeRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.args.is_empty() {
            return formatter.write_str(&self.name);
        }
        write!(formatter, "{}<", self.name)?;
        for (index, argument) in self.args.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{argument}")?;
        }
        formatter.write_str(">")
    }
}

/// Everything the analyzer can resolve about the crate under construction.
#[derive(Default)]
pub(crate) struct Index {
    functions: HashMap<String, ItemFn>,
    methods: HashMap<(String, String), ItemFn>,
    into_response: HashMap<String, syn::ImplItemFn>,
    structs: HashMap<String, ItemStruct>,
    enums: HashMap<String, ItemEnum>,
    aliases: HashMap<String, ItemType>,
    consts: HashMap<String, Type>,
}

impl Index {
    /// Index every Rust file under `src`.
    pub(crate) fn load(root: &Path) -> Self {
        let mut index = Self::default();
        let mut files = Vec::new();
        collect_rs_files(root, &mut files);
        for file in files {
            let Ok(source) = fs::read_to_string(&file) else {
                continue;
            };
            let Ok(parsed) = syn::parse_file(&source) else {
                continue;
            };
            index.insert_items(&parsed.items);
        }
        index
    }

    /// Index one module tree, recursively following inline modules.
    pub(crate) fn insert_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(function) => {
                    self.functions
                        .entry(function.sig.ident.to_string())
                        .or_insert_with(|| function.clone());
                }
                Item::Impl(item_impl) => self.insert_impl(item_impl),
                Item::Struct(structure) => {
                    self.structs
                        .entry(structure.ident.to_string())
                        .or_insert_with(|| structure.clone());
                }
                Item::Enum(item_enum) => {
                    self.enums
                        .entry(item_enum.ident.to_string())
                        .or_insert_with(|| item_enum.clone());
                }
                Item::Type(alias) => {
                    self.aliases
                        .entry(alias.ident.to_string())
                        .or_insert_with(|| alias.clone());
                }
                Item::Const(ItemConst { ident, ty, .. })
                | Item::Static(ItemStatic { ident, ty, .. }) => {
                    self.consts
                        .entry(ident.to_string())
                        .or_insert_with(|| (**ty).clone());
                }
                Item::Mod(module) => {
                    if let Some((_, items)) = &module.content {
                        self.insert_items(items);
                    }
                }
                _ => {}
            }
        }
    }

    fn insert_impl(&mut self, item_impl: &ItemImpl) {
        let Some(self_type) = type_ref(&item_impl.self_ty) else {
            return;
        };
        let trait_name = item_impl.trait_.as_ref().map(|(_, path, _)| {
            path.segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default()
        });

        if trait_name.as_deref() == Some("IntoResponse") {
            let method = item_impl.items.iter().find_map(|item| match item {
                ImplItem::Fn(function) if function.sig.ident == "into_response" => {
                    Some(function.clone())
                }
                _ => None,
            });
            if let Some(method) = method {
                self.into_response
                    .entry(self_type.name.clone())
                    .or_insert(method);
            }
        }

        for item in &item_impl.items {
            let ImplItem::Fn(function) = item else {
                continue;
            };
            if trait_name.is_none() {
                self.methods
                    .entry((self_type.name.clone(), function.sig.ident.to_string()))
                    .or_insert_with(|| impl_to_item_fn(function));
            }
        }
    }

    /// A free function by name.
    pub(crate) fn function(&self, name: &str) -> Option<ItemFn> {
        self.functions.get(name).cloned()
    }

    /// A free function's declared return type with aliases resolved.
    pub(crate) fn function_return(&self, name: &str) -> Option<TypeRef> {
        return_type(self.functions.get(name)?, &self.aliases)
    }

    /// An inherent method by receiver type and name.
    pub(crate) fn method(&self, type_name: &str, method: &str) -> Option<ItemFn> {
        let key = (type_name.to_string(), method.to_string());
        self.methods.get(&key).cloned()
    }

    /// The `IntoResponse::into_response` implementation of a type.
    pub(crate) fn response_impl(&self, type_name: &str) -> Option<syn::ImplItemFn> {
        self.into_response.get(type_name).cloned()
    }

    /// A named struct field's type.
    pub(crate) fn field_type(&self, type_name: &str, field: &str) -> Option<TypeRef> {
        let structure = self.structs.get(type_name)?;
        let Fields::Named(fields) = &structure.fields else {
            return None;
        };
        let field = fields
            .named
            .iter()
            .find(|candidate| candidate.ident.as_ref().is_some_and(|ident| ident == field))?;
        normalize_type(&field.ty, &self.aliases)
    }

    /// A type with aliases resolved.
    pub(crate) fn type_ref(&self, ty: &Type) -> Option<TypeRef> {
        normalize_type(ty, &self.aliases)
    }

    /// A type alias target, following aliases transitively.
    pub(crate) fn resolve_alias(&self, type_ref: &TypeRef) -> TypeRef {
        let Some(alias) = self.aliases.get(&type_ref.name) else {
            return type_ref.clone();
        };
        normalize_type(&alias.ty, &self.aliases).unwrap_or_else(|| type_ref.clone())
    }

    /// Infer the type of an expression, using the local type environment.
    pub(crate) fn infer_type(
        &self,
        expression: &Expr,
        locals: &HashMap<String, TypeRef>,
    ) -> Option<TypeRef> {
        match expression {
            Expr::Path(path) => {
                if path.path.segments.len() == 1 {
                    let name = path.path.segments[0].ident.to_string();
                    if let Some(local) = locals.get(&name) {
                        return Some(local.clone());
                    }
                    if let Some(ty) = self.consts.get(&name) {
                        return normalize_type(ty, &self.aliases);
                    }
                    if self.structs.contains_key(&name) || self.enums.contains_key(&name) {
                        return Some(TypeRef::new(name));
                    }
                }
                let name = path.path.segments.last()?.ident.to_string();
                match name.as_str() {
                    "None" => Some(TypeRef::with_args("Option", vec![TypeRef::new("Unknown")])),
                    "true" | "false" => Some(TypeRef::new("bool")),
                    _ if name.chars().next().is_some_and(char::is_uppercase) => {
                        Some(TypeRef::new(name))
                    }
                    _ => None,
                }
            }
            Expr::Struct(structure) => {
                let name = structure.path.segments.last()?.ident.to_string();
                Some(TypeRef::new(name))
            }
            Expr::Call(call) => self.infer_call(call, locals),
            Expr::MethodCall(call) => self.infer_method(call, locals),
            Expr::Field(field) => {
                let base = self.infer_type(&field.base, locals)?;
                let syn::Member::Named(name) = &field.member else {
                    return None;
                };
                self.field_type(&base.name, &name.to_string())
            }
            Expr::Await(await_expression) => self.infer_type(&await_expression.base, locals),
            Expr::Try(try_expression) => {
                let value = self.infer_type(&try_expression.expr, locals)?;
                match value.name.as_str() {
                    "Result" | "Option" => value.arg(0).cloned(),
                    _ => Some(value),
                }
            }
            Expr::Paren(paren) => self.infer_type(&paren.expr, locals),
            Expr::Group(group) => self.infer_type(&group.expr, locals),
            Expr::Reference(reference) => self.infer_type(&reference.expr, locals),
            Expr::Unary(unary) => self.infer_type(&unary.expr, locals),
            Expr::Cast(cast) => normalize_type(&cast.ty, &self.aliases),
            Expr::If(if_expression) => {
                let then_type = self.infer_block(&if_expression.then_branch, locals);
                let else_type = if_expression
                    .else_branch
                    .as_ref()
                    .and_then(|(_, expression)| self.infer_type(expression, locals));
                merge_types(then_type, else_type)
            }
            Expr::Match(match_expression) => {
                let mut merged = None;
                for arm in &match_expression.arms {
                    merged = merge_types(merged, self.infer_type(&arm.body, locals));
                }
                merged
            }
            Expr::Block(block) => self.infer_block(&block.block, locals),
            Expr::Lit(literal) => match &literal.lit {
                syn::Lit::Str(_) => Some(TypeRef::new("str")),
                syn::Lit::ByteStr(_) => Some(TypeRef::new("Bytes")),
                syn::Lit::Bool(_) => Some(TypeRef::new("bool")),
                _ => None,
            },
            Expr::Tuple(tuple) if tuple.elems.len() == 1 => {
                self.infer_type(tuple.elems.first()?, locals)
            }
            _ => None,
        }
    }

    fn infer_block(
        &self,
        block: &syn::Block,
        locals: &HashMap<String, TypeRef>,
    ) -> Option<TypeRef> {
        let mut locals = locals.clone();
        for statement in &block.stmts {
            if let syn::Stmt::Local(local) = statement
                && let Some((name, ty)) = local_binding(local, self, &locals)
            {
                locals.insert(name, ty);
            }
        }
        match block.stmts.last()? {
            syn::Stmt::Expr(expression, None) => self.infer_type(expression, &locals),
            _ => None,
        }
    }

    fn infer_call(
        &self,
        call: &syn::ExprCall,
        locals: &HashMap<String, TypeRef>,
    ) -> Option<TypeRef> {
        let Expr::Path(function) = call.func.as_ref() else {
            return None;
        };
        let segments: Vec<String> = function
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        let name = segments.last()?.as_str();

        if matches!(name, "Ok" | "Some") {
            let inner = call
                .args
                .first()
                .and_then(|argument| self.infer_type(argument, locals))
                .unwrap_or_else(|| TypeRef::new("Unknown"));
            let outer = if name == "Ok" { "Result" } else { "Option" };
            return Some(TypeRef::with_args(
                outer,
                vec![inner, TypeRef::new("Unknown")],
            ));
        }
        if name == "Err" {
            let inner = call
                .args
                .first()
                .and_then(|argument| self.infer_type(argument, locals))
                .unwrap_or_else(|| TypeRef::new("Unknown"));
            return Some(TypeRef::with_args(
                "Result",
                vec![TypeRef::new("Unknown"), inner],
            ));
        }

        if matches!(name, "Json" | "Created") {
            let inner = call
                .args
                .first()
                .and_then(|argument| self.infer_type(argument, locals))
                .unwrap_or_else(|| TypeRef::new("Unknown"));
            return Some(TypeRef::with_args(name, vec![inner]));
        }

        if let Some(type_ref) = function_type_ref(&function.path, &self.aliases) {
            // `Json::<T>(value)` carries the schema in its turbofish.
            if matches!(
                type_ref.name.as_str(),
                "Json" | "Created" | "Vec" | "Option"
            ) && type_ref.arg(0).is_some_and(|arg| !arg.is("Unknown"))
            {
                return Some(type_ref);
            }
            return Some(type_ref);
        }

        if segments.len() >= 2 {
            let receiver = &segments[segments.len() - 2];
            if let Some(method) = self.method(receiver, name) {
                return return_type(&method, &self.aliases);
            }
            if let Some(item_enum) = self.enums.get(receiver)
                && item_enum
                    .variants
                    .iter()
                    .any(|variant| variant.ident == name)
            {
                return Some(TypeRef::new(receiver.clone()));
            }
        }

        let function = self.function(name)?;
        return_type(&function, &self.aliases)
    }

    fn infer_method(
        &self,
        call: &syn::ExprMethodCall,
        locals: &HashMap<String, TypeRef>,
    ) -> Option<TypeRef> {
        let receiver = self.infer_type(&call.receiver, locals);
        let method = call.method.to_string();
        let receiver = receiver.map(|receiver| self.resolve_alias(&receiver));

        match method.as_str() {
            "into_response" => return Some(TypeRef::new("Response")),
            "clone" | "to_owned" => return receiver,
            "to_string" => return Some(TypeRef::new("String")),
            "as_str" => return Some(TypeRef::new("str")),
            "unwrap" | "expect" | "unwrap_or" | "unwrap_or_else" | "unwrap_or_default" => {
                let receiver = receiver?;
                return match receiver.name.as_str() {
                    "Option" | "Result" => receiver.arg(0).cloned(),
                    _ => Some(receiver),
                };
            }
            "map" | "map_err" | "and_then" | "or_else" => {
                let receiver = receiver?;
                let argument = closure_or_function_type(call.args.first(), &receiver, self);
                return match receiver.name.as_str() {
                    "Option" | "Result" if method == "map" => {
                        Some(TypeRef::with_args(receiver.name, vec![argument?]))
                    }
                    "Result" if method == "map_err" => Some(TypeRef::with_args(
                        "Result",
                        vec![
                            receiver
                                .arg(0)
                                .cloned()
                                .unwrap_or_else(|| TypeRef::new("Unknown")),
                            argument?,
                        ],
                    )),
                    _ => argument,
                };
            }
            "ok_or" | "ok_or_else" => {
                let receiver = receiver?;
                let error = call
                    .args
                    .first()
                    .and_then(|argument| closure_or_function_type(Some(argument), &receiver, self))
                    .or_else(|| match call.args.first() {
                        Some(argument) => self.infer_type(argument, locals),
                        None => None,
                    })
                    .unwrap_or_else(|| TypeRef::new("Unknown"));
                return Some(TypeRef::with_args(
                    "Result",
                    vec![
                        receiver
                            .arg(0)
                            .cloned()
                            .unwrap_or_else(|| TypeRef::new("Unknown")),
                        error,
                    ],
                ));
            }
            "get" | "first" | "last" | "pop"
                if receiver.as_ref().is_some_and(|receiver| {
                    matches!(receiver.name.as_str(), "Option" | "Result" | "Vec")
                }) =>
            {
                let receiver = receiver?;
                let inner = match receiver.name.as_str() {
                    "Option" | "Result" | "Vec" => receiver.arg(0)?.clone(),
                    _ => return None,
                };
                return Some(TypeRef::with_args("Option", vec![inner]));
            }
            _ => {}
        }

        let receiver = receiver?;
        if let Some(method) = self.method(&receiver.name, &method) {
            return return_type(&method, &self.aliases);
        }
        None
    }
}

fn impl_to_item_fn(function: &syn::ImplItemFn) -> ItemFn {
    ItemFn {
        attrs: function.attrs.clone(),
        vis: syn::Visibility::Inherited,
        sig: function.sig.clone(),
        block: Box::new(function.block.clone()),
    }
}

fn merge_types(left: Option<TypeRef>, right: Option<TypeRef>) -> Option<TypeRef> {
    match (left, right) {
        (Some(left), Some(right)) if left == right => Some(left),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        _ => None,
    }
}

fn closure_or_function_type(
    expression: Option<&Expr>,
    receiver: &TypeRef,
    index: &Index,
) -> Option<TypeRef> {
    let expression = expression?;
    match expression {
        Expr::Closure(closure) => {
            let mut locals = HashMap::new();
            if let Some(parameter) = closure.inputs.first()
                && let Pat::Ident(ident) = parameter
                && let Some(inner) = receiver.arg(0)
            {
                locals.insert(ident.ident.to_string(), inner.clone());
            }
            match &*closure.body {
                Expr::Block(block) => index.infer_block(&block.block, &locals),
                body => index.infer_type(body, &locals),
            }
        }
        Expr::Path(path) => {
            let name = path.path.segments.last()?.ident.to_string();
            // Constructors are functions whose schema comes from the value they wrap.
            if matches!(name.as_str(), "Json" | "Created") {
                return Some(TypeRef::with_args(name, vec![receiver.arg(0)?.clone()]));
            }
            let function = index.function(&name)?;
            return_type(&function, &index.aliases)
        }
        _ => index.infer_type(expression, &HashMap::new()),
    }
}

/// Bind a local pattern to its type, reading annotations or the initializer.
pub(crate) fn local_binding(
    local: &syn::Local,
    index: &Index,
    locals: &HashMap<String, TypeRef>,
) -> Option<(String, TypeRef)> {
    match &local.pat {
        Pat::Type(pat) => {
            let Pat::Ident(ident) = pat.pat.as_ref() else {
                return None;
            };
            Some((
                ident.ident.to_string(),
                normalize_type(&pat.ty, &index.aliases)?,
            ))
        }
        Pat::Ident(ident) => {
            let init = local.init.as_ref()?;
            let ty = index.infer_type(&init.expr, locals)?;
            Some((ident.ident.to_string(), ty))
        }
        _ => None,
    }
}

/// Bind handler arguments: `State(app)` yields `app: AppState`, `Json(body)` yields the body type.
pub(crate) fn bind_argument(
    pattern: &Pat,
    ty: &Type,
    index: &Index,
    locals: &mut HashMap<String, TypeRef>,
) {
    let Some(ty) = normalize_type(ty, &index.aliases) else {
        return;
    };
    match pattern {
        Pat::Ident(ident) => {
            locals.insert(ident.ident.to_string(), ty);
        }
        Pat::TupleStruct(tuple) => {
            let inner = match ty.name.as_str() {
                "State" | "Json" | "Path" | "Query" | "Strict" | "Mapped" | "Extension" => {
                    ty.arg(0).cloned().unwrap_or(ty)
                }
                _ => ty,
            };
            if let Some(element) = tuple.elems.first() {
                bind_pattern(element, &inner, index, locals);
            }
        }
        Pat::Struct(structure) => {
            let Some(structure_type) = index.structs.get(&ty.name) else {
                return;
            };
            for field in &structure.fields {
                let syn::Member::Named(name) = &field.member else {
                    continue;
                };
                let Some(field_type) =
                    index.field_type(&structure_type.ident.to_string(), &name.to_string())
                else {
                    continue;
                };
                if let Pat::Ident(ident) = field.pat.as_ref() {
                    locals.insert(ident.ident.to_string(), field_type);
                }
            }
        }
        Pat::Reference(reference) => bind_pattern(&reference.pat, &ty, index, locals),
        Pat::Paren(paren) => bind_pattern(&paren.pat, &ty, index, locals),
        _ => {}
    }
}

fn bind_pattern(pattern: &Pat, ty: &TypeRef, index: &Index, locals: &mut HashMap<String, TypeRef>) {
    match pattern {
        Pat::Ident(ident) => {
            locals.insert(ident.ident.to_string(), ty.clone());
        }
        Pat::Struct(structure) => {
            let Some(structure_type) = index.structs.get(&ty.name) else {
                return;
            };
            for field in &structure.fields {
                let syn::Member::Named(name) = &field.member else {
                    continue;
                };
                let Some(field_type) =
                    index.field_type(&structure_type.ident.to_string(), &name.to_string())
                else {
                    continue;
                };
                if let Pat::Ident(ident) = field.pat.as_ref() {
                    locals.insert(ident.ident.to_string(), field_type);
                }
            }
        }
        Pat::TupleStruct(tuple) => {
            if let Some(element) = tuple.elems.first() {
                bind_pattern(element, ty, index, locals);
            }
        }
        Pat::Reference(reference) => bind_pattern(&reference.pat, ty, index, locals),
        Pat::Paren(paren) => bind_pattern(&paren.pat, ty, index, locals),
        _ => {}
    }
}

/// A function or method's declared return type.
pub(crate) fn return_type(
    function: &ItemFn,
    aliases: &HashMap<String, ItemType>,
) -> Option<TypeRef> {
    let ReturnType::Type(_, ty) = &function.sig.output else {
        return None;
    };
    normalize_type(ty, aliases)
}

/// Normalize a syntax type into a [`TypeRef`], following aliases.
pub(crate) fn normalize_type(ty: &Type, aliases: &HashMap<String, ItemType>) -> Option<TypeRef> {
    let mut resolved = type_ref(ty)?;
    // Follow simple aliases, guarding against a cycle.
    for _ in 0..8 {
        let Some(alias) = aliases.get(&resolved.name) else {
            break;
        };
        let Some(next) = type_ref(&alias.ty) else {
            break;
        };
        resolved = next;
    }
    Some(resolved)
}

fn type_ref(ty: &Type) -> Option<TypeRef> {
    match ty {
        Type::Path(path) => {
            let segment = path.path.segments.last()?;
            let args = match &segment.arguments {
                syn::PathArguments::None => Vec::new(),
                syn::PathArguments::AngleBracketed(arguments) => arguments
                    .args
                    .iter()
                    .filter_map(|argument| match argument {
                        syn::GenericArgument::Type(ty) => type_ref(ty),
                        _ => None,
                    })
                    .collect(),
                syn::PathArguments::Parenthesized(_) => Vec::new(),
            };
            Some(TypeRef::with_args(segment.ident.to_string(), args))
        }
        Type::Reference(reference) => type_ref(&reference.elem),
        Type::Paren(paren) => type_ref(&paren.elem),
        Type::Group(group) => type_ref(&group.elem),
        Type::Slice(slice) => Some(TypeRef::with_args("Vec", vec![type_ref(&slice.elem)?])),
        Type::Array(array) => Some(TypeRef::with_args("Vec", vec![type_ref(&array.elem)?])),
        Type::Tuple(tuple) if tuple.elems.len() == 1 => type_ref(tuple.elems.first()?),
        _ => None,
    }
}

/// The outer type of a path used as a constructor, including turbofish arguments.
fn function_type_ref(path: &syn::Path, aliases: &HashMap<String, ItemType>) -> Option<TypeRef> {
    let segment = path.segments.last()?;
    let name = segment.ident.to_string();
    match &segment.arguments {
        syn::PathArguments::None => None,
        syn::PathArguments::AngleBracketed(arguments) => {
            let args: Vec<TypeRef> = arguments
                .args
                .iter()
                .filter_map(|argument| match argument {
                    syn::GenericArgument::Type(ty) => normalize_type(ty, aliases),
                    _ => None,
                })
                .collect();
            if args.is_empty() {
                return None;
            }
            Some(TypeRef::with_args(name, args))
        }
        syn::PathArguments::Parenthesized(_) => None,
    }
}

fn collect_rs_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}
