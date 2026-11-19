//! Local semantic index for the handler analyzer.
//!
//! The analyzer has no compiler session, so this module resolves the parts of Rust that matter for
//! selecting an opaque handler's response type: helper and method return types, struct fields,
//! type aliases, consts and local `let` bindings. It is deliberately conservative: an unresolved
//! type is `None`, never a guess. Statuses, schemas and headers do not live here; the response
//! traits own them.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::{
    Expr, Fields, FnArg, GenericArgument, GenericParam, ImplItem, Item, ItemConst, ItemEnum,
    ItemFn, ItemImpl, ItemStatic, ItemStruct, ItemType, Pat, ReturnType, Type,
};

use roundhouse_schema::status_codes::{self as status_catalog, StatusKind};

/// A syntactically resolved type: its outer name, any const arguments and any type arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TypeRef {
    pub name: String,
    pub consts: Vec<String>,
    pub args: Vec<TypeRef>,
}

impl TypeRef {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            consts: Vec::new(),
            args: Vec::new(),
        }
    }

    pub(crate) fn with_args(name: impl Into<String>, args: Vec<TypeRef>) -> Self {
        Self {
            name: name.into(),
            consts: Vec::new(),
            args,
        }
    }

    pub(crate) fn with_consts(
        name: impl Into<String>,
        consts: Vec<String>,
        args: Vec<TypeRef>,
    ) -> Self {
        Self {
            name: name.into(),
            consts,
            args,
        }
    }

    pub(crate) fn is(&self, name: &str) -> bool {
        self.name == name
    }

    pub(crate) fn arg(&self, index: usize) -> Option<&TypeRef> {
        self.args.get(index)
    }
}

impl std::fmt::Display for TypeRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.consts.is_empty() && self.args.is_empty() {
            return formatter.write_str(&self.name);
        }
        write!(formatter, "{}<", self.name)?;
        let mut first = true;
        for constant in &self.consts {
            if !first {
                formatter.write_str(", ")?;
            }
            formatter.write_str(constant)?;
            first = false;
        }
        for argument in &self.args {
            if !first {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{argument}")?;
            first = false;
        }
        formatter.write_str(">")
    }
}

/// Everything the analyzer can resolve about the crate under construction.
#[derive(Default)]
pub(crate) struct Index {
    functions: HashMap<String, ItemFn>,
    handlers: Vec<ItemFn>,
    methods: HashMap<(String, String), ItemFn>,
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
                    if function.attrs.iter().any(|attribute| {
                        attribute
                            .path()
                            .segments
                            .last()
                            .is_some_and(|segment| segment.ident == "handler")
                    }) {
                        self.handlers.push(function.clone());
                    }
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

    /// Every function carrying a `#[handler]` attribute.
    pub(crate) fn handlers(&self) -> &[ItemFn] {
        &self.handlers
    }

    /// A free function by name.
    pub(crate) fn function(&self, name: &str) -> Option<ItemFn> {
        self.functions.get(name).cloned()
    }

    /// A function or method's declared return type with aliases resolved.
    pub(crate) fn item_return(&self, function: &ItemFn) -> Option<TypeRef> {
        return_type(function, &self.aliases)
    }

    /// Resolve a call path to the function or inherent method it names.
    pub(crate) fn callee(&self, path: &syn::Path) -> Option<ItemFn> {
        let segments: Vec<&syn::PathSegment> = path.segments.iter().collect();
        let name = segments.last()?.ident.to_string();
        if segments.len() >= 2 {
            let receiver = segments[segments.len() - 2].ident.to_string();
            if let Some(method) = self.methods.get(&(receiver, name.clone())) {
                return Some(method.clone());
            }
        }
        self.function(&name)
    }

    /// An inherent method by receiver type and name.
    pub(crate) fn method(&self, type_name: &str, method: &str) -> Option<ItemFn> {
        let key = (type_name.to_string(), method.to_string());
        self.methods.get(&key).cloned()
    }

    /// Whether the crate defines a type with this name.
    ///
    /// The analyzer only sees names, not imports. A local definition takes precedence over the
    /// generated wrappers when an opaque return type happens to share a name such as `Conflict`.
    pub(crate) fn has_local_type(&self, name: &str) -> bool {
        self.structs.contains_key(name)
            || self.enums.contains_key(name)
            || self.aliases.contains_key(name)
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

    /// A type alias target, following aliases transitively.
    pub(crate) fn resolve_alias(&self, type_ref: &TypeRef) -> TypeRef {
        normalize_type_ref(type_ref, &self.aliases).unwrap_or_else(|| type_ref.clone())
    }

    /// A function or method's return type, with type parameters substituted from explicit
    /// arguments and the call's argument types.
    ///
    /// The analyzer has no compiler session, so a generic helper's return type is only usable when
    /// every type parameter is known. Anything still generic becomes unresolved instead of leaking
    /// a name such as `T` into the generated documentation call.
    fn call_return<'a>(
        &self,
        function: &ItemFn,
        explicit: Option<&syn::PathSegment>,
        arguments: impl IntoIterator<Item = &'a Expr>,
        locals: &HashMap<String, TypeRef>,
    ) -> Option<TypeRef> {
        let ty = return_type(function, &self.aliases)?;
        let parameters = generic_type_params(&function.sig.generics);
        if parameters.is_empty() {
            return Some(ty);
        }

        let mut substitutions = HashMap::new();
        if let Some(segment) = explicit
            && let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments
        {
            let explicit: Vec<&Type> = arguments
                .args
                .iter()
                .filter_map(|argument| match argument {
                    GenericArgument::Type(ty) => Some(ty),
                    _ => None,
                })
                .collect();
            for (parameter, ty) in parameters.iter().zip(explicit) {
                if let Some(ty) = normalize_type(ty, &self.aliases) {
                    substitutions.insert(parameter.clone(), ty);
                }
            }
        }

        // A method's receiver is not part of `arguments`, so skip it when pairing parameter types.
        let mut arguments = arguments.into_iter();
        for input in &function.sig.inputs {
            let FnArg::Typed(parameter) = input else {
                continue;
            };
            let Some(argument) = arguments.next() else {
                break;
            };
            let Some(argument) = self.infer_type(argument, locals) else {
                continue;
            };
            if let Some(parameter) = normalize_type(&parameter.ty, &self.aliases) {
                unify_type_ref(&parameter, &argument, &parameters, &mut substitutions);
            }
        }

        let substituted = substitute_type_ref(&ty, &substitutions)?;
        if has_generic_name(&substituted, &parameters) {
            return None;
        }
        Some(substituted)
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
                let last = path.path.segments.last()?;
                // Const-generic values such as `Empty::<204>` are response types in their own
                // right. Other generic paths (constructor functions) are left to the caller.
                if let syn::PathArguments::AngleBracketed(arguments) = &last.arguments
                    && arguments
                        .args
                        .iter()
                        .any(|argument| matches!(argument, syn::GenericArgument::Const(_)))
                {
                    return Some(type_ref_from_segment(last));
                }
                let name = last.ident.to_string();
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
            Expr::Tuple(tuple) if tuple.elems.is_empty() => Some(TypeRef::new("()")),
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
        let Expr::Path(callee_path) = call.func.as_ref() else {
            return None;
        };
        let segments: Vec<String> = callee_path
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

        if segments.len() >= 2
            && segments[segments.len() - 2] == "String"
            && matches!(name, "from" | "new")
        {
            return Some(TypeRef::new("String"));
        }

        // Generated named wrappers: `Created(value)`, `Created::new(value)`, and the empty
        // variants. The catalog is the same data that generated the types.
        if let Some(spec) = status_catalog::by_ident(&segments[0])
            && !self.has_local_type(&segments[0])
            && (name == "new" || name == spec.ident)
        {
            return match spec.kind {
                StatusKind::Body => {
                    let inner = named_segment(&callee_path.path, &spec.ident)
                        .and_then(type_argument)
                        .or_else(|| {
                            call.args
                                .first()
                                .and_then(|argument| self.infer_type(argument, locals))
                        })
                        .unwrap_or_else(|| TypeRef::new("Unknown"));
                    Some(TypeRef::with_args(&spec.ident, vec![inner]))
                }
                StatusKind::Empty => Some(TypeRef::new(&spec.ident)),
            };
        }

        // `Status::<201, _>::new(value)` and `Empty::<204>::new()` carry their code in the
        // turbofish; the return type cannot be named by inference alone because the code is a
        // const parameter.
        if segments[0] == "Status" && !self.has_local_type("Status") {
            let segment = callee_path
                .path
                .segments
                .iter()
                .find(|segment| segment.ident == "Status")?;
            let consts = const_argument(segment)?;
            let inner = type_argument(segment)
                .or_else(|| {
                    call.args
                        .first()
                        .and_then(|argument| self.infer_type(argument, locals))
                })
                .unwrap_or_else(|| TypeRef::new("Unknown"));
            return Some(TypeRef::with_consts("Status", vec![consts], vec![inner]));
        }
        if segments[0] == "Empty" && !self.has_local_type("Empty") {
            let segment = callee_path
                .path
                .segments
                .iter()
                .find(|segment| segment.ident == "Empty")?;
            let consts = const_argument(segment)?;
            return Some(TypeRef::with_consts("Empty", vec![consts], Vec::new()));
        }

        if matches!(name, "Json" | "Html") {
            let inner = call
                .args
                .first()
                .and_then(|argument| self.infer_type(argument, locals))
                .unwrap_or_else(|| TypeRef::new("Unknown"));
            return Some(TypeRef::with_args(name, vec![inner]));
        }

        // A generic helper's return type is only known after substituting its parameters. Do this
        // before the constructor-turbofish path so `load::<Note>(..)` resolves like `load(..)`.
        if let Some(callee) = self.callee(&callee_path.path)
            && !generic_type_params(&callee.sig.generics).is_empty()
        {
            return self.call_return(
                &callee,
                callee_path.path.segments.last(),
                call.args.iter(),
                locals,
            );
        }

        if let Some(type_ref) = function_type_ref(&callee_path.path, &self.aliases) {
            // `Json::<T>(value)` carries the schema in its turbofish.
            if matches!(type_ref.name.as_str(), "Json" | "Vec" | "Option")
                && type_ref.arg(0).is_some_and(|arg| !arg.is("Unknown"))
            {
                return Some(type_ref);
            }
            return Some(type_ref);
        }

        if segments.len() >= 2 {
            let receiver = &segments[segments.len() - 2];
            if let Some(method) = self.method(receiver, name) {
                return self.call_return(
                    &method,
                    callee_path.path.segments.last(),
                    call.args.iter(),
                    locals,
                );
            }
            if receiver == "Problem" {
                // `Problem`'s associated constructors (`not_found`, `bad_request`, …) live in
                // the roundhouse crate, which this local index does not parse, and every one
                // returns `Problem`.
                return Some(TypeRef::new("Problem"));
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
        self.call_return(
            &function,
            callee_path.path.segments.last(),
            call.args.iter(),
            locals,
        )
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
            // Both erasure methods are transparent to the document: the receiver's type is the
            // source of truth.
            "into_response" | "into_documented_response" => return receiver,
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
            return self.call_return(&method, None, call.args.iter(), locals);
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
            return index.call_return(&function, None, std::iter::empty(), &HashMap::new());
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
        // `let Some(note) = .. else { .. }` and `let Ok(note) = .. else { .. }` bind the
        // success payload; the analyzer needs it to resolve the tail.
        Pat::TupleStruct(tuple) => {
            let init = local.init.as_ref()?;
            let ty = index.infer_type(&init.expr, locals)?;
            let variant = tuple.path.segments.last()?.ident.to_string();
            let inner = match (variant.as_str(), ty.name.as_str()) {
                ("Some", "Option") | ("Ok", "Result") => ty.arg(0)?.clone(),
                _ => return None,
            };
            let Pat::Ident(ident) = tuple.elems.first()? else {
                return None;
            };
            Some((ident.ident.to_string(), inner))
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
    let resolved = type_ref(ty)?;
    normalize_type_ref(&resolved, aliases)
}

/// Follow aliases on an already-normalized type, substituting the alias's type arguments.
///
/// `type ApiResult<T> = Result<T, Problem>` used as `ApiResult<Page>` must become
/// `Result<Page, Problem>`; keeping the alias's own `T` would leak a name the generated
/// documentation code cannot resolve.
fn normalize_type_ref(resolved: &TypeRef, aliases: &HashMap<String, ItemType>) -> Option<TypeRef> {
    let mut resolved = resolved.clone();
    // Follow aliases, guarding against a cycle.
    for _ in 0..8 {
        let Some(alias) = aliases.get(&resolved.name) else {
            break;
        };
        let target = type_ref(&alias.ty)?;
        let parameters = generic_type_params(&alias.generics);
        if parameters.is_empty() {
            resolved = target;
            continue;
        }
        let substitutions: HashMap<String, TypeRef> = parameters
            .iter()
            .cloned()
            .zip(resolved.args.iter().cloned())
            .collect();
        if parameters
            .iter()
            .any(|parameter| !substitutions.contains_key(parameter))
        {
            return None;
        }
        resolved = substitute_type_ref(&target, &substitutions)?;
    }
    Some(resolved)
}

/// The type parameter names declared by a function, method, or alias.
fn generic_type_params(generics: &syn::Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .filter_map(|parameter| match parameter {
            GenericParam::Type(parameter) => Some(parameter.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// Replace type parameters in a resolved type.
fn substitute_type_ref(ty: &TypeRef, substitutions: &HashMap<String, TypeRef>) -> Option<TypeRef> {
    if let Some(replacement) = substitutions.get(&ty.name) {
        return Some(replacement.clone());
    }
    let args = ty
        .args
        .iter()
        .map(|argument| substitute_type_ref(argument, substitutions))
        .collect::<Option<Vec<_>>>()?;
    Some(TypeRef::with_consts(
        ty.name.clone(),
        ty.consts.clone(),
        args,
    ))
}

fn has_generic_name(ty: &TypeRef, parameters: &[String]) -> bool {
    parameters.iter().any(|parameter| parameter == &ty.name)
        || ty
            .args
            .iter()
            .any(|argument| has_generic_name(argument, parameters))
}

/// Infer a generic parameter from a declared parameter type and a call argument type.
fn unify_type_ref(
    parameter: &TypeRef,
    argument: &TypeRef,
    parameters: &[String],
    substitutions: &mut HashMap<String, TypeRef>,
) {
    if parameters.iter().any(|name| name == &parameter.name) {
        substitutions
            .entry(parameter.name.clone())
            .or_insert_with(|| argument.clone());
        return;
    }
    if parameter.name != argument.name {
        return;
    }
    for (parameter, argument) in parameter.args.iter().zip(argument.args.iter()) {
        unify_type_ref(parameter, argument, parameters, substitutions);
    }
}

fn type_ref(ty: &Type) -> Option<TypeRef> {
    match ty {
        Type::Path(path) => Some(type_ref_from_segment(path.path.segments.last()?)),
        Type::Reference(reference) => type_ref(&reference.elem),
        Type::Paren(paren) => type_ref(&paren.elem),
        Type::Group(group) => type_ref(&group.elem),
        Type::Slice(slice) => Some(TypeRef::with_args("Vec", vec![type_ref(&slice.elem)?])),
        Type::Array(array) => Some(TypeRef::with_args("Vec", vec![type_ref(&array.elem)?])),
        Type::Tuple(tuple) if tuple.elems.len() == 1 => type_ref(tuple.elems.first()?),
        _ => None,
    }
}

fn type_ref_from_segment(segment: &syn::PathSegment) -> TypeRef {
    let mut consts = Vec::new();
    let mut args = Vec::new();
    if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
        for argument in &arguments.args {
            match argument {
                syn::GenericArgument::Type(ty) => {
                    if let Some(ty) = type_ref(ty) {
                        args.push(ty);
                    }
                }
                syn::GenericArgument::Const(expression) => {
                    consts.push(expression.to_token_stream().to_string());
                }
                _ => {}
            }
        }
    }
    TypeRef::with_consts(segment.ident.to_string(), consts, args)
}

fn named_segment<'a>(path: &'a syn::Path, name: &str) -> Option<&'a syn::PathSegment> {
    path.segments.iter().find(|segment| segment.ident == name)
}

pub(crate) fn const_argument(segment: &syn::PathSegment) -> Option<String> {
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        syn::GenericArgument::Const(expression) => Some(expression.to_token_stream().to_string()),
        _ => None,
    })
}

pub(crate) fn type_argument(segment: &syn::PathSegment) -> Option<TypeRef> {
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        syn::GenericArgument::Type(ty) => type_ref(ty),
        _ => None,
    })
}

/// The outer type of a path used as a constructor, including turbofish arguments.
fn function_type_ref(path: &syn::Path, aliases: &HashMap<String, ItemType>) -> Option<TypeRef> {
    let segment = path.segments.last()?;
    let name = segment.ident.to_string();
    match &segment.arguments {
        syn::PathArguments::None => None,
        syn::PathArguments::AngleBracketed(arguments) => {
            let mut consts = Vec::new();
            let mut args = Vec::new();
            for argument in &arguments.args {
                match argument {
                    syn::GenericArgument::Type(ty) => {
                        if let Some(ty) = normalize_type(ty, aliases) {
                            args.push(ty);
                        }
                    }
                    syn::GenericArgument::Const(expression) => {
                        consts.push(expression.to_token_stream().to_string());
                    }
                    _ => {}
                }
            }
            if consts.is_empty() && args.is_empty() {
                return None;
            }
            Some(TypeRef::with_consts(name, consts, args))
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
