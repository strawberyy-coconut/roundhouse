//! Conservative source response inference.
//!
//! The analyzer runs after a discovery contract build. It reads each operation's handler source,
//! walks the return expressions, and records known statuses/content types plus a body component
//! name when it is syntactically visible. Anything it cannot classify becomes a `default`
//! `Problem` response instead of a guess.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use syn::{Expr, FnArg, Item, ItemFn, ReturnType, Signature, Stmt};

use crate::semantic::{Index, TypeRef, bind_argument, local_binding};

#[derive(Default)]
struct AnalyzeContext {
    index: Index,
    visiting: HashSet<String>,
    locals: HashMap<String, TypeRef>,
    /// The error type of the function currently being analyzed, used to resolve `?`.
    error_type: Option<TypeRef>,
    /// Guards against a response type whose `IntoResponse` body refers back to itself.
    analyzing_types: HashSet<String>,
}

impl AnalyzeContext {
    fn load_crate(&mut self) {
        self.index = Index::load(Path::new("src"));
    }

    #[cfg(test)]
    fn insert_items(&mut self, items: &[Item]) {
        self.index.insert_items(items);
    }

    fn function(&self, name: &str) -> Option<ItemFn> {
        self.index.function(name)
    }
}

#[derive(Clone, Debug)]
struct Inferred {
    status: u16,
    content_type: Option<String>,
    schema: Option<String>,
    description: Option<String>,
    /// The analyzer could not classify this return path; the build treats it as an unknown
    /// `default Problem` that strict mode rejects unless the author acknowledges it.
    unknown: bool,
}

impl Inferred {
    fn new(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            schema: None,
            description: None,
            unknown: false,
        }
    }

    fn unknown() -> Self {
        let mut response = Self::problem(0);
        response.unknown = true;
        response
    }

    fn json(status: u16) -> Self {
        Self::new(status).content_type("application/json")
    }

    fn problem(status: u16) -> Self {
        Self::new(status)
            .content_type("application/problem+json")
            .schema("Problem")
    }

    fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }

    fn schema(mut self, schema: impl Into<String>) -> Self {
        self.schema = Some(schema.into());
        self
    }

    fn to_json(&self) -> Value {
        let mut value = Map::new();
        value.insert("status".to_string(), json!(self.status));
        if let Some(content_type) = &self.content_type {
            value.insert("content_type".to_string(), json!(content_type));
        }
        if let Some(schema) = &self.schema {
            value.insert("schema".to_string(), json!(schema));
        }
        if let Some(description) = &self.description {
            value.insert("description".to_string(), json!(description));
        }
        if self.unknown {
            value.insert("unknown".to_string(), json!(true));
        }
        Value::Object(value)
    }
}

/// Analyze the discovery artifacts and write the inference document.
pub fn analyze(discovery: &Path, output: &Path) -> Result<(), Box<dyn Error>> {
    let contract: Value = read_json(&discovery.join("contract.json"))?;
    let openapi: Value = read_json(&discovery.join("openapi.json"))?;
    let operations = contract["operations"]
        .as_array()
        .ok_or_else(|| error("discovery contract has no operations"))?;

    let mut context = AnalyzeContext::default();
    context.load_crate();
    let mut inferred = Map::new();
    for operation in operations {
        let Some(operation_id) = operation["operation_id"].as_str() else {
            continue;
        };
        let Some(path) = operation["path"].as_str() else {
            continue;
        };
        let Some(method) = operation["method"].as_str() else {
            continue;
        };
        let method = method.to_ascii_lowercase();
        if has_success(&openapi, path, &method) {
            continue;
        }

        let Some(handler) = operation["handler"].as_str() else {
            continue;
        };
        let Some(source_file) = operation["source"]["file"].as_str() else {
            continue;
        };
        let Some(function) = find_function(Path::new(source_file), handler) else {
            eprintln!(
                "warning: could not find handler `{handler}` for `{operation_id}`; leaving its responses unverified"
            );
            continue;
        };

        let mut responses = Vec::new();
        analyze_function(&function, &mut responses, &mut context);
        if responses.is_empty() || responses.iter().all(|response| response.unknown) {
            eprintln!(
                "warning: operation `{operation_id}` ({method} {path}) has an outcome the analyzer \
                 could not classify; add `#[handler(response(..))]`, `@response default: ...`, or \
                 a route-level `unverified = \"reason\"`"
            );
        }
        if responses.is_empty() {
            responses.push(Inferred::unknown());
        }
        inferred.insert(
            operation_id.to_string(),
            Value::Array(dedupe(responses).iter().map(Inferred::to_json).collect()),
        );
    }

    let inference = json!({ "operations": inferred });
    let pretty = serde_json::to_string_pretty(&inference)?;
    fs::write(output, format!("{pretty}\n"))?;
    Ok(())
}

fn has_success(openapi: &Value, path: &str, method: &str) -> bool {
    let Some(responses) = openapi["paths"][path][method]["responses"].as_object() else {
        return false;
    };
    responses.iter().any(|(status, response)| {
        if status == "default" {
            // The discovery build inserts this placeholder for operations whose success is not
            // documented yet. A real default response carries content or custom prose.
            response["description"].as_str() != Some("Default Response")
                || response.get("content").is_some()
        } else {
            status
                .parse::<u16>()
                .is_ok_and(|status| (200..400).contains(&status))
        }
    })
}

fn find_function(path: &Path, name: &str) -> Option<ItemFn> {
    if let Ok(source) = fs::read_to_string(path)
        && let Ok(file) = syn::parse_file(&source)
        && let Some(function) = find_in_items(&file.items, name)
    {
        return Some(function.clone());
    }

    let mut files = Vec::new();
    collect_rs_files(Path::new("src"), &mut files);
    for file in files {
        if let Ok(source) = fs::read_to_string(&file)
            && let Ok(parsed) = syn::parse_file(&source)
            && let Some(function) = find_in_items(&parsed.items, name)
        {
            return Some(function.clone());
        }
    }
    None
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

fn find_in_items<'a>(items: &'a [Item], name: &str) -> Option<&'a ItemFn> {
    for item in items {
        match item {
            Item::Fn(function) if function.sig.ident == name => return Some(function),
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content
                    && let Some(function) = find_in_items(items, name)
                {
                    return Some(function);
                }
            }
            _ => {}
        }
    }
    None
}

/// Analyze a free function as a response producer.
fn analyze_function(function: &ItemFn, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    analyze_signature_block(
        &function.sig,
        &function.block,
        out,
        context,
        Some(&function.sig.ident.to_string()),
    );
}

/// Analyze an `IntoResponse` implementation body as a response producer.
fn analyze_impl_method(
    function: &syn::ImplItemFn,
    out: &mut Vec<Inferred>,
    context: &mut AnalyzeContext,
) {
    analyze_signature_block(&function.sig, &function.block, out, context, None);
}

fn analyze_signature_block(
    signature: &Signature,
    block: &syn::Block,
    out: &mut Vec<Inferred>,
    context: &mut AnalyzeContext,
    visiting: Option<&String>,
) {
    let saved_locals = context.locals.clone();
    let saved_error = context.error_type.clone();
    context.locals.clear();
    for input in &signature.inputs {
        if let FnArg::Typed(pat_type) = input {
            bind_argument(
                &pat_type.pat,
                &pat_type.ty,
                &context.index,
                &mut context.locals,
            );
        }
    }
    context.error_type = function_error(signature, &context.index);
    analyze_block(block, out, context);
    context.locals = saved_locals;
    context.error_type = saved_error;
    if let Some(name) = visiting {
        context.visiting.remove(name);
    }
}

fn function_error(signature: &Signature, index: &Index) -> Option<TypeRef> {
    let ReturnType::Type(_, ty) = &signature.output else {
        return None;
    };
    let ty = index.type_ref(ty)?;
    if ty.is("Result") {
        ty.arg(1).cloned()
    } else {
        None
    }
}

fn analyze_block(block: &syn::Block, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    let saved_locals = context.locals.clone();

    for statement in &block.stmts {
        if let Stmt::Local(local) = statement {
            if let Some((name, ty)) = local_type(local, context) {
                context.locals.insert(name, ty);
            }
            if let Some(init) = &local.init {
                scan_try(&init.expr, out, context);
                scan_returns(&init.expr, out, context);
            }
        }
    }

    let count = block.stmts.len();
    for (index, statement) in block.stmts.iter().enumerate() {
        match statement {
            Stmt::Expr(expression, None) if index + 1 == count => {
                scan_try(expression, out, context);
                analyze_value(expression, out, context);
            }
            Stmt::Expr(expression, _) => {
                scan_try(expression, out, context);
                scan_returns(expression, out, context);
            }
            _ => {}
        }
    }

    context.locals = saved_locals;
}

fn local_type(local: &syn::Local, context: &AnalyzeContext) -> Option<(String, TypeRef)> {
    local_binding(local, &context.index, &context.locals)
}

fn scan_returns(expression: &Expr, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    match expression {
        Expr::Return(returned) => match &returned.expr {
            Some(value) => analyze_value(value, out, context),
            None => out.push(Inferred::new(200)),
        },
        Expr::Block(block) => analyze_block(&block.block, out, context),
        Expr::If(if_expression) => {
            analyze_block(&if_expression.then_branch, out, context);
            if let Some((_, else_expression)) = &if_expression.else_branch {
                scan_returns(else_expression, out, context);
            }
        }
        Expr::Match(match_expression) => {
            for arm in &match_expression.arms {
                scan_returns(&arm.body, out, context);
            }
        }
        Expr::Try(try_expression) => analyze_try(try_expression, out, context),
        Expr::Await(await_expression) => scan_returns(&await_expression.base, out, context),
        Expr::Paren(paren) => scan_returns(&paren.expr, out, context),
        Expr::Group(group) => scan_returns(&group.expr, out, context),
        Expr::Unary(unary) => scan_returns(&unary.expr, out, context),
        Expr::Reference(reference) => scan_returns(&reference.expr, out, context),
        Expr::Cast(cast) => scan_returns(&cast.expr, out, context),
        Expr::Call(call) => {
            for argument in &call.args {
                scan_returns(argument, out, context);
            }
        }
        Expr::MethodCall(call) => {
            scan_returns(&call.receiver, out, context);
            for argument in &call.args {
                scan_returns(argument, out, context);
            }
        }
        Expr::Binary(binary) => {
            scan_returns(&binary.left, out, context);
            scan_returns(&binary.right, out, context);
        }
        Expr::Loop(loop_expression) => analyze_block(&loop_expression.body, out, context),
        Expr::While(while_expression) => analyze_block(&while_expression.body, out, context),
        Expr::ForLoop(for_loop) => analyze_block(&for_loop.body, out, context),
        _ => {}
    }
}

/// Add the error path of every `?` in an expression.
fn scan_try(expression: &Expr, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    match expression {
        Expr::Try(try_expression) => {
            analyze_try(try_expression, out, context);
            scan_try(&try_expression.expr, out, context);
        }
        Expr::Block(block) => {
            for statement in &block.block.stmts {
                match statement {
                    Stmt::Local(local) => {
                        if let Some(init) = &local.init {
                            scan_try(&init.expr, out, context);
                        }
                    }
                    Stmt::Expr(expression, _) => scan_try(expression, out, context),
                    _ => {}
                }
            }
        }
        Expr::If(if_expression) => {
            for statement in &if_expression.then_branch.stmts {
                if let Stmt::Expr(expression, _) = statement {
                    scan_try(expression, out, context);
                }
            }
            if let Some((_, else_expression)) = &if_expression.else_branch {
                scan_try(else_expression, out, context);
            }
        }
        Expr::Match(match_expression) => {
            for arm in &match_expression.arms {
                scan_try(&arm.body, out, context);
            }
        }
        Expr::Await(await_expression) => scan_try(&await_expression.base, out, context),
        Expr::Paren(paren) => scan_try(&paren.expr, out, context),
        Expr::Group(group) => scan_try(&group.expr, out, context),
        Expr::Unary(unary) => scan_try(&unary.expr, out, context),
        Expr::Reference(reference) => scan_try(&reference.expr, out, context),
        Expr::Cast(cast) => scan_try(&cast.expr, out, context),
        Expr::Call(call) => {
            for argument in &call.args {
                scan_try(argument, out, context);
            }
        }
        Expr::MethodCall(call) => {
            scan_try(&call.receiver, out, context);
            for argument in &call.args {
                scan_try(argument, out, context);
            }
        }
        Expr::Binary(binary) => {
            scan_try(&binary.left, out, context);
            scan_try(&binary.right, out, context);
        }
        Expr::Tuple(tuple) => {
            for element in &tuple.elems {
                scan_try(element, out, context);
            }
        }
        // A `?` inside a closure belongs to the closure's error type, not the handler's.
        Expr::Closure(_) => {}
        _ => {}
    }
}

/// Record the error path of one `?`.
///
/// Error constructors written inline (`load().ok_or_else(|| Problem::not_found(..))?`) are more
/// precise than the declared error type, so they win. Otherwise the enclosing function's error
/// type is analyzed through its `IntoResponse` implementation.
fn analyze_try(
    try_expression: &syn::ExprTry,
    out: &mut Vec<Inferred>,
    context: &mut AnalyzeContext,
) {
    let mut inner = Vec::new();
    analyze_value(&try_expression.expr, &mut inner, context);
    // An unclassified success expression says nothing about the `?` error path; the declared
    // error type is the better signal there.
    inner.retain(|response| !response.unknown && (response.status == 0 || response.status >= 400));
    let found = !inner.is_empty();
    out.extend(inner);
    if !found {
        analyze_error_type(out, context);
    }
}

fn analyze_error_type(out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    if let Some(error) = context.error_type.clone() {
        if error.is("Problem") {
            out.push(Inferred::problem(500));
            return;
        }
        if analyze_response_type(&error, out, context) {
            return;
        }
    }
    out.push(Inferred::problem(500));
}

fn analyze_value(expression: &Expr, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    match expression {
        Expr::Return(returned) => match &returned.expr {
            Some(value) => analyze_value(value, out, context),
            None => out.push(Inferred::new(200)),
        },
        Expr::Paren(paren) => analyze_value(&paren.expr, out, context),
        Expr::Group(group) => analyze_value(&group.expr, out, context),
        Expr::Reference(reference) => analyze_value(&reference.expr, out, context),
        Expr::Unary(unary) => analyze_value(&unary.expr, out, context),
        Expr::Cast(cast) => analyze_value(&cast.expr, out, context),
        Expr::Try(try_expression) => analyze_try(try_expression, out, context),
        Expr::Await(await_expression) => analyze_value(&await_expression.base, out, context),
        Expr::MethodCall(call) => analyze_method_call(expression, call, out, context),
        Expr::Lit(literal) => match &literal.lit {
            syn::Lit::Str(_) => {
                out.push(Inferred::new(200).content_type("text/plain; charset=utf-8"))
            }
            syn::Lit::ByteStr(_) => {
                out.push(Inferred::new(200).content_type("application/octet-stream"))
            }
            _ => out.push(Inferred::unknown()),
        },
        Expr::Call(call) => analyze_call(call, out, context),
        Expr::Struct(structure) => {
            let ty = TypeRef::new(
                structure
                    .path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string())
                    .unwrap_or_default(),
            );
            if !analyze_response_type(&ty, out, context) {
                out.push(Inferred::unknown());
            }
        }
        Expr::Tuple(tuple) => analyze_tuple(tuple, out, context),
        Expr::Match(match_expression) => {
            for arm in &match_expression.arms {
                analyze_value(&arm.body, out, context);
            }
        }
        Expr::If(if_expression) => {
            analyze_block(&if_expression.then_branch, out, context);
            match &if_expression.else_branch {
                Some((_, else_expression)) => analyze_value(else_expression, out, context),
                // Without an else branch the other path returns `()`, which is a 200 with no body.
                None => out.push(Inferred::new(200)),
            }
        }
        Expr::Block(block) => analyze_block(&block.block, out, context),
        Expr::Path(path) => {
            if let Some(response) = response_from_path(&path.path) {
                out.push(response);
                return;
            }
            let ty = context.index.infer_type(expression, &context.locals);
            if ty.is_some_and(|ty| analyze_response_type(&ty, out, context)) {
                return;
            }
            out.push(Inferred::unknown());
        }
        _ => out.push(Inferred::unknown()),
    }
}

fn analyze_method_call(
    expression: &Expr,
    call: &syn::ExprMethodCall,
    out: &mut Vec<Inferred>,
    context: &mut AnalyzeContext,
) {
    if let Some(status) = status_from_response_chain(expression) {
        out.push(Inferred::new(status));
        return;
    }

    match call.method.to_string().as_str() {
        "into_response" | "unwrap" | "expect" | "header" | "body" | "content_type" | "status" => {
            analyze_value(&call.receiver, out, context);
        }
        "ok_or" | "ok_or_else" | "unwrap_or" | "unwrap_or_else" => {
            analyze_success_path(&call.receiver, out, context);
            if let Some(argument) = call.args.first() {
                analyze_error_path(argument, out, context);
            }
        }
        "map" | "and_then" => {
            analyze_map_output(&call.receiver, call.args.first(), out, context);
        }
        "map_err" | "or_else" => {
            analyze_value(&call.receiver, out, context);
            if let Some(argument) = call.args.first() {
                analyze_error_path(argument, out, context);
            }
        }
        _ => {
            let before = out.len();
            // A fluent builder such as `Problem::not_found(..).detail(..)` keeps the base
            // constructor's outcome.
            analyze_value(&call.receiver, out, context);
            if out[before..].iter().all(|response| response.unknown) {
                out.truncate(before);
                if let Some(ty) = context.index.infer_type(expression, &context.locals)
                    && analyze_response_type(&ty, out, context)
                {
                    return;
                }
                out.push(Inferred::unknown());
            }
        }
    }
}

/// The successful half of an `Option`/`Result` chain: `.map(Json)` produces the `Json` body.
fn analyze_success_path(expression: &Expr, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    match expression {
        Expr::MethodCall(call)
            if matches!(call.method.to_string().as_str(), "map" | "and_then") =>
        {
            analyze_map_output(&call.receiver, call.args.first(), out, context);
        }
        Expr::Call(call) => {
            let Expr::Path(function) = call.func.as_ref() else {
                analyze_value(expression, out, context);
                return;
            };
            let name = function
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default();
            if matches!(name.as_str(), "Some" | "Ok") {
                if let Some(argument) = call.args.first() {
                    analyze_value(argument, out, context);
                }
                return;
            }
            analyze_value(expression, out, context);
        }
        _ => {
            if let Some(ty) = context.index.infer_type(expression, &context.locals) {
                let inner = match ty.name.as_str() {
                    "Option" | "Result" => ty.arg(0).cloned(),
                    _ => Some(ty),
                };
                if inner.is_some_and(|inner| analyze_response_type(&inner, out, context)) {
                    return;
                }
            }
            analyze_value(expression, out, context);
        }
    }
}

/// Analyze the mapped value of `.map(..)`/`.and_then(..)` as a response.
fn analyze_map_output(
    receiver: &Expr,
    producer: Option<&Expr>,
    out: &mut Vec<Inferred>,
    context: &mut AnalyzeContext,
) {
    let inner = context
        .index
        .infer_type(receiver, &context.locals)
        .and_then(|ty| match ty.name.as_str() {
            "Option" | "Result" => ty.arg(0).cloned(),
            _ => Some(ty),
        });

    let Some(producer) = producer else {
        return;
    };
    match producer {
        Expr::Path(path) => {
            let name = path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default();
            match name.as_str() {
                "Json" => {
                    let mut response = Inferred::json(200);
                    response.schema = inner.as_ref().and_then(TypeRef::schema_name);
                    out.push(response);
                }
                "Created" => {
                    let mut response = Inferred::json(201);
                    response.schema = inner.as_ref().and_then(TypeRef::schema_name);
                    out.push(response);
                }
                _ => {
                    if let Some(function) = context.function(&name) {
                        analyze_function(&function, out, context);
                    } else if let Some(ty) = context.index.function_return(&name) {
                        if !analyze_response_type(&ty, out, context) {
                            out.push(Inferred::unknown());
                        }
                    } else {
                        out.push(Inferred::unknown());
                    }
                }
            }
        }
        Expr::Closure(closure) => {
            let saved_locals = context.locals.clone();
            if let (Some(parameter), Some(inner)) = (closure.inputs.first(), inner.as_ref())
                && let syn::Pat::Ident(ident) = parameter
            {
                context
                    .locals
                    .insert(ident.ident.to_string(), inner.clone());
            }
            match &*closure.body {
                Expr::Block(block) => analyze_block(&block.block, out, context),
                body => analyze_value(body, out, context),
            }
            context.locals = saved_locals;
        }
        _ => analyze_value(producer, out, context),
    }
}

/// Analyze an error path: a closure, an error constructor, or a function that returns one.
fn analyze_error_path(expression: &Expr, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    match expression {
        Expr::Closure(closure) => match &*closure.body {
            Expr::Block(block) => analyze_block(&block.block, out, context),
            body => analyze_value(body, out, context),
        },
        _ => analyze_value(expression, out, context),
    }
}

/// Document a type's response shape without an expression to inspect.
fn analyze_response_type(
    ty: &TypeRef,
    out: &mut Vec<Inferred>,
    context: &mut AnalyzeContext,
) -> bool {
    let ty = context.index.resolve_alias(ty);
    match ty.name.as_str() {
        "Json" => {
            let mut response = Inferred::json(200);
            response.schema = ty.arg(0).and_then(TypeRef::schema_name);
            out.push(response);
            true
        }
        "Created" => {
            let mut response = Inferred::json(201);
            response.schema = ty.arg(0).and_then(TypeRef::schema_name);
            out.push(response);
            true
        }
        "NoContent" => {
            out.push(Inferred::new(204));
            true
        }
        "Html" => {
            out.push(Inferred::new(200).content_type("text/html"));
            true
        }
        "Bytes" | "Body" => {
            out.push(Inferred::new(200).content_type("application/octet-stream"));
            true
        }
        "String" | "str" => {
            out.push(Inferred::new(200).content_type("text/plain; charset=utf-8"));
            true
        }
        "StatusCode" => {
            out.push(Inferred::new(0));
            true
        }
        "Problem" => {
            out.push(Inferred::problem(0));
            true
        }
        "Option" => ty
            .arg(0)
            .is_some_and(|inner| analyze_response_type(inner, out, context)),
        "Result" => {
            let ok = ty
                .arg(0)
                .is_some_and(|inner| analyze_response_type(inner, out, context));
            let err = ty
                .arg(1)
                .is_some_and(|inner| analyze_response_type(inner, out, context));
            ok || err
        }
        "Response" | "Unknown" | "Infallible" => false,
        name if name.chars().next().is_some_and(char::is_uppercase) => {
            if !context.analyzing_types.insert(name.to_string()) {
                return false;
            }
            let Some(method) = context.index.response_impl(name) else {
                context.analyzing_types.remove(name);
                return false;
            };
            let before = out.len();
            analyze_impl_method(&method, out, context);
            context.analyzing_types.remove(name);
            out.len() > before
        }
        _ => false,
    }
}

fn analyze_call(call: &syn::ExprCall, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    let Expr::Path(function) = call.func.as_ref() else {
        out.push(Inferred::unknown());
        return;
    };
    let segments: Vec<String> = function
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let name = segments.last().map(String::as_str).unwrap_or_default();

    match (segments.len(), name) {
        (_, "Ok") | (_, "Some") => {
            if let Some(argument) = call.args.first() {
                analyze_value(argument, out, context);
            }
        }
        (_, "Err") => {
            if let Some(argument) = call.args.first() {
                analyze_value(argument, out, context);
            } else {
                out.push(Inferred::unknown());
            }
        }
        (_, "Json") | (_, "JsonValue") => {
            let mut response = Inferred::json(200);
            response.schema = type_argument(function.path.segments.last()).or_else(|| {
                call.args
                    .first()
                    .and_then(|expr| context.index.infer_type(expr, &context.locals))
                    .and_then(|ty| ty.schema_name())
            });
            out.push(response);
        }
        (_, "Created") => {
            let status = 201;
            if let Some(argument) = call.args.first() {
                let mut inner = Vec::new();
                analyze_value(argument, &mut inner, context);
                if let Some(mut response) = inner.into_iter().find(|response| !response.unknown) {
                    response.status = status;
                    out.push(response);
                    return;
                }
            }
            let mut response = Inferred::json(status);
            response.schema = call
                .args
                .first()
                .and_then(|argument| context.index.infer_type(argument, &context.locals))
                .and_then(|ty| ty.schema_name());
            out.push(response);
        }
        (_, "NoContent") => out.push(Inferred::new(204)),
        (_, "Html") => out.push(Inferred::new(200).content_type("text/html")),
        (_, "Bytes") | (_, "Body") => {
            out.push(Inferred::new(200).content_type("application/octet-stream"))
        }
        (_, "to")
            if segments
                .first()
                .is_some_and(|segment| segment == "Redirect") =>
        {
            out.push(Inferred::new(303))
        }
        (_, "temporary")
            if segments
                .first()
                .is_some_and(|segment| segment == "Redirect") =>
        {
            out.push(Inferred::new(307))
        }
        (_, "permanent")
            if segments
                .first()
                .is_some_and(|segment| segment == "Redirect") =>
        {
            out.push(Inferred::new(308))
        }
        (_, "from_validation") if segments.first().is_some_and(|s| s == "Problem") => {
            out.push(Inferred::problem(400));
        }
        (_, constructor) if segments.first().is_some_and(|s| s == "Problem") => {
            let status = problem_status(constructor)
                .unwrap_or_else(|| call.args.first().and_then(status_from_expr).unwrap_or(500));
            let mut response = Inferred::problem(status);
            response.description = call.args.iter().find_map(string_literal);
            out.push(response);
        }
        _ => {
            if let Some(function) = context.function(name)
                && context.visiting.insert(name.to_string())
            {
                analyze_function(&function, out, context);
            } else if let Some(ty) = context
                .index
                .infer_type(&Expr::Call(call.clone()), &context.locals)
            {
                if !analyze_response_type(&ty, out, context) {
                    out.push(Inferred::unknown());
                }
            } else {
                out.push(Inferred::unknown());
            }
        }
    }
}

fn analyze_tuple(tuple: &syn::ExprTuple, out: &mut Vec<Inferred>, context: &mut AnalyzeContext) {
    if let Some(first) = tuple.elems.first()
        && let Some(status) = status_from_expr(first)
    {
        if let Some(body) = tuple.elems.get(1) {
            let mut inner = Vec::new();
            analyze_value(body, &mut inner, context);
            if let Some(mut response) = inner.into_iter().find(|response| !response.unknown) {
                response.status = status;
                if matches!(status, 204 | 304) {
                    response.content_type = None;
                    response.schema = None;
                }
                out.push(response);
                return;
            }
            let mut response = Inferred::new(status);
            if !matches!(status, 204 | 304) {
                response.schema = context
                    .index
                    .infer_type(body, &context.locals)
                    .and_then(|ty| ty.schema_name());
            }
            out.push(response);
            return;
        }
        out.push(Inferred::new(status));
        return;
    }
    out.push(Inferred::unknown());
}

fn response_from_path(path: &syn::Path) -> Option<Inferred> {
    let name = path.segments.last()?.ident.to_string();
    if name == "NoContent" {
        return Some(Inferred::new(204));
    }
    status_from_path(path).map(Inferred::new)
}

fn status_from_expr(expression: &Expr) -> Option<u16> {
    match expression {
        Expr::Path(path) => status_from_path(&path.path),
        Expr::Lit(literal) => match &literal.lit {
            syn::Lit::Int(value) => value
                .base10_parse::<u16>()
                .ok()
                .filter(|status| (100..600).contains(status)),
            _ => None,
        },
        Expr::Paren(paren) => status_from_expr(&paren.expr),
        Expr::Group(group) => status_from_expr(&group.expr),
        Expr::Reference(reference) => status_from_expr(&reference.expr),
        Expr::Call(call) => {
            let Expr::Path(path) = call.func.as_ref() else {
                return None;
            };
            if path.path.segments.last()?.ident == "from_u16" {
                match call.args.first()? {
                    Expr::Lit(literal) => match &literal.lit {
                        syn::Lit::Int(value) => value.base10_parse().ok(),
                        _ => None,
                    },
                    _ => None,
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Find an explicit `.status(...)` in a response-builder method chain.
fn status_from_response_chain(expression: &Expr) -> Option<u16> {
    match expression {
        Expr::MethodCall(call) => {
            if call.method == "status" {
                return call.args.first().and_then(status_from_expr);
            }
            status_from_response_chain(&call.receiver)
        }
        Expr::Await(await_expression) => status_from_response_chain(&await_expression.base),
        Expr::Try(try_expression) => status_from_response_chain(&try_expression.expr),
        Expr::Paren(paren) => status_from_response_chain(&paren.expr),
        Expr::Group(group) => status_from_response_chain(&group.expr),
        _ => None,
    }
}

fn status_from_path(path: &syn::Path) -> Option<u16> {
    let segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    if segments.len() < 2 || segments[segments.len() - 2] != "StatusCode" {
        return None;
    }
    status_from_name(segments.last()?.as_str())
}

fn status_from_name(name: &str) -> Option<u16> {
    Some(match name {
        "CONTINUE" => 100,
        "OK" => 200,
        "CREATED" => 201,
        "ACCEPTED" => 202,
        "NO_CONTENT" => 204,
        "MOVED_PERMANENTLY" => 301,
        "FOUND" => 302,
        "NOT_MODIFIED" => 304,
        "BAD_REQUEST" => 400,
        "UNAUTHORIZED" => 401,
        "FORBIDDEN" => 403,
        "NOT_FOUND" => 404,
        "METHOD_NOT_ALLOWED" => 405,
        "CONFLICT" => 409,
        "GONE" => 410,
        "UNPROCESSABLE_ENTITY" => 422,
        "TOO_MANY_REQUESTS" => 429,
        "INTERNAL_SERVER_ERROR" => 500,
        "NOT_IMPLEMENTED" => 501,
        "BAD_GATEWAY" => 502,
        "SERVICE_UNAVAILABLE" => 503,
        "GATEWAY_TIMEOUT" => 504,
        _ => return None,
    })
}

fn problem_status(constructor: &str) -> Option<u16> {
    Some(match constructor {
        "bad_request" => 400,
        "unauthorized" => 401,
        "forbidden" => 403,
        "not_found" => 404,
        "conflict" => 409,
        "unprocessable_entity" => 422,
        "too_many_requests" => 429,
        "internal" => 500,
        "service_unavailable" => 503,
        _ => return None,
    })
}

fn string_literal(expression: &Expr) -> Option<String> {
    let Expr::Lit(literal) = expression else {
        return None;
    };
    match &literal.lit {
        syn::Lit::Str(value) => Some(value.value()),
        _ => None,
    }
}

fn type_argument(segment: Option<&syn::PathSegment>) -> Option<String> {
    let segment = segment?;
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(ty) = arguments.args.first()? else {
        return None;
    };
    simple_type_name(ty)
}

fn simple_type_name(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if !matches!(segment.arguments, syn::PathArguments::None) {
        return None;
    }
    Some(segment.ident.to_string())
}

fn dedupe(responses: Vec<Inferred>) -> Vec<Inferred> {
    let mut deduped: Vec<Inferred> = Vec::new();
    for response in responses {
        if let Some(existing) = deduped
            .iter_mut()
            .find(|existing| existing.status == response.status)
        {
            if existing.schema.is_none() && response.schema.is_some() {
                existing.schema = response.schema;
            }
            if existing.content_type.is_none() && response.content_type.is_some() {
                existing.content_type = response.content_type;
            }
            if existing.description.is_none() && response.description.is_some() {
                existing.description = response.description;
            }
        } else {
            deduped.push(response);
        }
    }
    deduped
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let source = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&source)?)
}

fn error(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze_source(source: &str) -> Vec<Inferred> {
        let file = syn::parse_file(source).expect("valid source");
        let function = find_in_items(&file.items, "handler").expect("handler");
        let mut context = AnalyzeContext::default();
        context.insert_items(&file.items);
        let mut responses = Vec::new();
        analyze_function(function, &mut responses, &mut context);
        dedupe(responses)
    }

    #[test]
    fn follows_local_helpers() {
        let responses = analyze_source(
            r#"
            fn missing() -> Response {
                Problem::not_found("gone").into_response()
            }

            async fn handler() -> impl IntoResponse {
                missing()
            }
            "#,
        );
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].status, 404);
        assert_eq!(responses[0].description.as_deref(), Some("gone"));
    }

    #[test]
    fn reads_helper_return_types_for_body_schemas() {
        let responses = analyze_source(
            r#"
            fn load_note() -> Hello {
                Hello { message: "note".to_string() }
            }

            async fn handler() -> impl IntoResponse {
                Json(load_note()).into_response()
            }
            "#,
        );
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].status, 200);
        assert_eq!(responses[0].schema.as_deref(), Some("Hello"));
    }

    #[test]
    fn adds_a_500_for_question_mark_and_reads_typed_locals() {
        let responses = analyze_source(
            r#"
            async fn handler() -> Result<Json<Hello>, Problem> {
                let note: Hello = load()?;
                Ok(Json(note))
            }
            "#,
        );
        let statuses: Vec<u16> = responses.iter().map(|response| response.status).collect();
        assert!(statuses.contains(&500));
        let success = responses
            .iter()
            .find(|response| response.status == 200)
            .expect("success");
        assert_eq!(success.schema.as_deref(), Some("Hello"));
    }

    #[test]
    fn infers_match_arms_and_into_response() {
        let responses = analyze_source(
            r#"
            async fn handler() -> impl IntoResponse {
                match load() {
                    Some(note) => Json(note).into_response(),
                    None => Problem::not_found("missing").into_response(),
                }
            }
            "#,
        );

        let statuses: Vec<u16> = responses.iter().map(|response| response.status).collect();
        assert_eq!(statuses, vec![200, 404]);
        assert_eq!(
            responses[0].content_type.as_deref(),
            Some("application/json")
        );
        assert_eq!(
            responses[1].content_type.as_deref(),
            Some("application/problem+json")
        );
        assert_eq!(responses[1].schema.as_deref(), Some("Problem"));
    }

    #[test]
    fn infers_created_and_no_content() {
        let responses = analyze_source(
            r#"
            async fn handler() -> impl IntoResponse {
                if create() {
                    Created(Json(note)).into_response()
                } else {
                    NoContent.into_response()
                }
            }
            "#,
        );
        let statuses: Vec<u16> = responses.iter().map(|response| response.status).collect();
        assert_eq!(statuses, vec![201, 204]);
    }

    #[test]
    fn infers_status_tuple_and_status_code() {
        let responses = analyze_source(
            r#"
            async fn handler() -> impl IntoResponse {
                if ok() {
                    (StatusCode::ACCEPTED, Json(note)).into_response()
                } else {
                    StatusCode::CONFLICT.into_response()
                }
            }
            "#,
        );
        let statuses: Vec<u16> = responses.iter().map(|response| response.status).collect();
        assert_eq!(statuses, vec![202, 409]);
    }

    #[test]
    fn infers_response_builder_chains_redirects_and_literals() {
        let builder = analyze_source(
            r#"
            async fn handler() -> impl IntoResponse {
                axum::response::Response::builder()
                    .status(StatusCode::CREATED)
                    .body(Body::empty())
                    .unwrap()
            }
            "#,
        );
        assert!(builder.iter().any(|response| response.status == 201));

        let redirect = analyze_source(
            r#"
            async fn handler() -> impl IntoResponse {
                Redirect::permanent("/moved")
            }
            "#,
        );
        assert!(redirect.iter().any(|response| response.status == 308));

        let text = analyze_source(
            r#"
            async fn handler() -> impl IntoResponse {
                if true { (StatusCode::ACCEPTED, "queued") } else { (StatusCode::OK, "done") }
            }
            "#,
        );
        assert!(text.iter().any(|response| {
            response.status == 202
                && response.content_type.as_deref() == Some("text/plain; charset=utf-8")
        }));
    }

    #[test]
    fn unknown_returns_default_problem() {
        let responses = analyze_source(
            r#"
            async fn handler() -> impl IntoResponse {
                opaque()
            }
            "#,
        );
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].status, 0);
        assert_eq!(responses[0].schema.as_deref(), Some("Problem"));
    }

    #[test]
    fn reads_method_return_types_for_body_schemas() {
        let responses = analyze_source(
            r#"
            struct Note {
                id: u64,
            }

            struct Store;

            impl Store {
                fn load(&self, id: u64) -> Note {
                    Note { id }
                }
            }

            async fn handler(store: Store) -> impl IntoResponse {
                let note = store.load(1);
                Json(note).into_response()
            }
            "#,
        );
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].status, 200);
        assert_eq!(responses[0].schema.as_deref(), Some("Note"));
    }

    #[test]
    fn follows_custom_into_response_implementations() {
        let responses = analyze_source(
            r#"
            struct Csv;

            impl IntoResponse for Csv {
                fn into_response(self) -> Response {
                    (StatusCode::OK, "id,name\n").into_response()
                }
            }

            async fn handler() -> Csv {
                Csv
            }
            "#,
        );
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].status, 200);
        assert_eq!(
            responses[0].content_type.as_deref(),
            Some("text/plain; charset=utf-8")
        );
    }

    #[test]
    fn resolves_option_chains_into_success_and_error() {
        let responses = analyze_source(
            r#"
            struct Note {
                id: u64,
            }

            struct Store;

            impl Store {
                fn get(&self, id: u64) -> Option<Note> {
                    None
                }
            }

            async fn handler(store: Store) -> impl IntoResponse {
                store
                    .get(1)
                    .map(Json)
                    .ok_or_else(|| Problem::not_found("missing"))
                    .into_response()
            }
            "#,
        );
        let statuses: Vec<u16> = responses.iter().map(|response| response.status).collect();
        assert_eq!(statuses, vec![200, 404]);
        assert_eq!(responses[0].schema.as_deref(), Some("Note"));
        assert_eq!(
            responses[1].content_type.as_deref(),
            Some("application/problem+json")
        );
    }

    #[test]
    fn maps_question_mark_through_a_custom_error_response() {
        let responses = analyze_source(
            r#"
            struct Note {
                id: u64,
            }

            struct Unavailable;

            impl IntoResponse for Unavailable {
                fn into_response(self) -> Response {
                    Problem::service_unavailable("down").into_response()
                }
            }

            fn load() -> Result<Note, Unavailable> {
                Ok(Note { id: 1 })
            }

            async fn handler() -> Result<Json<Note>, Unavailable> {
                let note = load()?;
                Ok(Json(note))
            }
            "#,
        );
        let statuses: Vec<u16> = responses.iter().map(|response| response.status).collect();
        assert!(statuses.contains(&200), "{statuses:?}");
        assert!(statuses.contains(&503), "{statuses:?}");
        let success = responses
            .iter()
            .find(|response| response.status == 200)
            .expect("success");
        assert_eq!(success.schema.as_deref(), Some("Note"));
    }
}
