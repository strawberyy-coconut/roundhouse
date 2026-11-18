//! Classify the success sites of an opaque handler body.
//!
//! The analyzer answers one question: which documented type leaves the handler at each tail or
//! `return`? It does not synthesize statuses or schemas — the traits do that when the generated
//! `DocSource` runs. Resolution is conservative: an expression it cannot classify becomes an
//! `unresolved` diagnostic, never a guess.

use std::collections::HashMap;

use quote::ToTokens;
use syn::{Expr, FnArg, ItemFn, ReturnType, Stmt, Type};

use super::ResponseSite;
use super::semantic::{Index, TypeRef, bind_argument, local_binding};

#[derive(Default)]
pub(crate) struct Analysis {
    pub sites: Vec<ResponseSite>,
    pub unresolved: Vec<String>,
}

/// Whether the return type hides a response behind `impl Trait`.
pub(crate) fn has_opaque_return(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };
    contains_impl_trait(ty)
}

pub(crate) fn analyze_handler(function: &ItemFn, index: &Index) -> Analysis {
    let mut analysis = Analysis::default();
    let mut locals = HashMap::new();
    for input in &function.sig.inputs {
        if let FnArg::Typed(argument) = input {
            bind_argument(&argument.pat, &argument.ty, index, &mut locals);
        }
    }
    let mut context = Context {
        index,
        locals,
        analysis: &mut analysis,
    };
    context.block(&function.block);
    dedupe(&mut analysis.sites);
    if analysis.sites.is_empty() && analysis.unresolved.is_empty() {
        analysis
            .unresolved
            .push("no documented success response was resolved".to_string());
    }
    analysis
}

struct Context<'a> {
    index: &'a Index,
    locals: HashMap<String, TypeRef>,
    analysis: &'a mut Analysis,
}

impl Context<'_> {
    fn block(&mut self, block: &syn::Block) {
        let saved = self.locals.clone();
        for statement in &block.stmts {
            if let Stmt::Local(local) = statement {
                if let Some((name, ty)) = local_binding(local, self.index, &self.locals) {
                    self.locals.insert(name, ty);
                }
                if let Some(init) = &local.init {
                    self.scan_returns(&init.expr);
                }
            }
        }
        let count = block.stmts.len();
        for (position, statement) in block.stmts.iter().enumerate() {
            match statement {
                Stmt::Expr(expression, None) if position + 1 == count => self.value(expression),
                Stmt::Expr(expression, _) => self.scan_returns(expression),
                _ => {}
            }
        }
        self.locals = saved;
    }

    fn value(&mut self, expression: &Expr) {
        match expression {
            Expr::Return(returned) => match &returned.expr {
                Some(value) => self.value(value),
                None => self.push_type("()"),
            },
            Expr::Paren(paren) => self.value(&paren.expr),
            Expr::Group(group) => self.value(&group.expr),
            Expr::Reference(reference) => self.value(&reference.expr),
            Expr::Unary(unary) => self.value(&unary.expr),
            Expr::Cast(cast) => self.value(&cast.expr),
            Expr::Await(awaited) => self.value(&awaited.base),
            // The `?` itself yields the success of its operand, not a response.
            Expr::Try(_) => {}
            Expr::MethodCall(call) => self.method_call(expression, call),
            Expr::Call(call) => self.call(expression, call),
            Expr::Struct(_) => self.type_site(expression),
            Expr::Path(path) => self.path_site(expression, &path.path),
            Expr::Lit(literal) => match &literal.lit {
                syn::Lit::Str(_) => self.push_type("&'static str"),
                _ => self.unresolved("a non-text literal response"),
            },
            Expr::Tuple(tuple) => self.tuple(tuple),
            Expr::Match(matched) => {
                for arm in &matched.arms {
                    self.value(&arm.body);
                }
            }
            Expr::If(if_expression) => {
                self.block(&if_expression.then_branch);
                match &if_expression.else_branch {
                    Some((_, else_expression)) => self.value(else_expression),
                    // Without an else branch the other path returns `()`.
                    None => self.push_type("()"),
                }
            }
            Expr::Block(block) => self.block(&block.block),
            Expr::Loop(loop_expression) => self.block(&loop_expression.body),
            Expr::While(while_expression) => self.block(&while_expression.body),
            Expr::ForLoop(for_loop) => self.block(&for_loop.body),
            Expr::Closure(_) => {}
            Expr::Macro(_) => self.unresolved("a macro-generated response"),
            _ => self.unresolved("an unsupported response expression"),
        }
    }

    fn method_call(&mut self, whole: &Expr, call: &syn::ExprMethodCall) {
        let method = call.method.to_string();
        match method.as_str() {
            // The user chose the erased response; do not pretend to know its shape, but keep the
            // operation honest when another branch did document a success.
            "into_response" => {
                self.unresolved("an opaque `.into_response()` chain");
            }
            // Explicit opt-in: document the receiver's type.
            "into_documented_response" => self.value(&call.receiver),
            _ => {
                // Prefer a method witness: it infers the return type from the method signature
                // and avoids naming a type that may not be imported in this module.
                if let Some(receiver) = self.index.infer_type(&call.receiver, &self.locals) {
                    let receiver = self.index.resolve_alias(&receiver);
                    if let Some(item) = self.index.method(&receiver.name, &method)
                        && item.sig.generics.params.is_empty()
                    {
                        if self
                            .index
                            .item_return(&item)
                            .is_some_and(|ty| ty.is("Response"))
                        {
                            self.unresolved("an opaque `Response` value");
                            return;
                        }
                        self.push_site(ResponseSite::Call {
                            path: format!("{}::{method}", receiver.name),
                            arity: callee_arity(&item),
                            is_async: item.sig.asyncness.is_some(),
                        });
                        return;
                    }
                }
                self.type_site(whole);
            }
        }
    }

    fn call(&mut self, whole: &Expr, call: &syn::ExprCall) {
        let Expr::Path(function) = call.func.as_ref() else {
            self.unresolved("a response call through a non-path callee");
            return;
        };
        let name = function
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();

        if name == "Ok" || name == "Some" {
            if let Some(argument) = call.args.first() {
                self.value(argument);
            }
            return;
        }
        if name == "Err" {
            // Errors are documented from the handler's `E`, not from each `Err(..)`.
            return;
        }
        let root = function
            .path
            .segments
            .first()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        match (root.as_str(), name.as_str()) {
            ("Created", "new") => {
                if let Some(ty) = call
                    .args
                    .first()
                    .and_then(|argument| self.index.infer_type(argument, &self.locals))
                {
                    self.push_type_ref(TypeRef::with_args("Created", vec![ty]));
                    return;
                }
            }
            ("Redirect", _) => {
                self.push_site(ResponseSite::Redirect);
                return;
            }
            ("StatusCode", _) => {
                self.push_site(ResponseSite::StatusCode);
                return;
            }
            ("String", "from") | ("String", "new") => {
                self.push_type("::std::string::String");
                return;
            }
            ("Sse", "new") => {
                // `Sse<E>`'s documentation does not depend on `E`.
                self.push_type("Sse<()>");
                return;
            }
            _ => {}
        }
        if is_constructor(&name) {
            self.type_site(whole);
            return;
        }

        if let Some(callee) = self.index.callee(&function.path)
            && callee.sig.generics.params.is_empty()
        {
            if self
                .index
                .item_return(&callee)
                .is_some_and(|ty| ty.is("Response"))
            {
                self.unresolved("an opaque `Response` value");
                return;
            }
            self.push_site(ResponseSite::Call {
                path: function.path.to_token_stream().to_string(),
                arity: callee_arity(&callee),
                is_async: callee.sig.asyncness.is_some(),
            });
            return;
        }

        self.type_site(whole);
    }

    fn path_site(&mut self, expression: &Expr, path: &syn::Path) {
        if is_status_path(path) {
            self.push_site(ResponseSite::StatusCode);
            return;
        }
        if path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Response")
        {
            // A bare `Response` has no recoverable shape.
            self.unresolved("a bare `Response` value");
            return;
        }
        if let Some(ty) = self.index.infer_type(expression, &self.locals) {
            self.push_type_ref(ty);
        } else {
            self.unresolved("a response path the analyzer cannot resolve");
        }
    }

    fn type_site(&mut self, expression: &Expr) {
        if let Some(ty) = self.index.infer_type(expression, &self.locals) {
            self.push_type_ref(ty);
        } else {
            self.unresolved("a response expression the analyzer cannot resolve");
        }
    }

    fn tuple(&mut self, tuple: &syn::ExprTuple) {
        if tuple.elems.len() == 1 {
            if let Some(element) = tuple.elems.first() {
                self.value(element);
            }
            return;
        }
        if tuple.elems.len() == 2 && is_status_expression(&tuple.elems[0]) {
            let inner = self
                .index
                .infer_type(&tuple.elems[1], &self.locals)
                .and_then(|ty| format_type(&ty));
            match inner {
                Some(inner) => self.push_site(ResponseSite::StatusTuple { inner }),
                None => self.unresolved("a status tuple whose body cannot be resolved"),
            }
            return;
        }
        self.unresolved("a response tuple");
    }

    fn scan_returns(&mut self, expression: &Expr) {
        match expression {
            Expr::Return(returned) => match &returned.expr {
                Some(value) => self.value(value),
                None => self.push_type("()"),
            },
            Expr::Block(block) => self.block(&block.block),
            Expr::If(if_expression) => {
                self.block(&if_expression.then_branch);
                if let Some((_, else_expression)) = &if_expression.else_branch {
                    self.scan_returns(else_expression);
                }
            }
            Expr::Match(matched) => {
                for arm in &matched.arms {
                    self.scan_returns(&arm.body);
                }
            }
            Expr::Try(tried) => self.scan_returns(&tried.expr),
            Expr::Await(awaited) => self.scan_returns(&awaited.base),
            Expr::Paren(paren) => self.scan_returns(&paren.expr),
            Expr::Group(group) => self.scan_returns(&group.expr),
            Expr::Unary(unary) => self.scan_returns(&unary.expr),
            Expr::Reference(reference) => self.scan_returns(&reference.expr),
            Expr::Cast(cast) => self.scan_returns(&cast.expr),
            Expr::Call(call) => {
                for argument in &call.args {
                    self.scan_returns(argument);
                }
            }
            Expr::MethodCall(call) => {
                self.scan_returns(&call.receiver);
                for argument in &call.args {
                    self.scan_returns(argument);
                }
            }
            Expr::Binary(binary) => {
                self.scan_returns(&binary.left);
                self.scan_returns(&binary.right);
            }
            Expr::Loop(loop_expression) => self.block(&loop_expression.body),
            Expr::While(while_expression) => self.block(&while_expression.body),
            Expr::ForLoop(for_loop) => self.block(&for_loop.body),
            // A `return` inside a closure belongs to the closure.
            Expr::Closure(_) => {}
            _ => {}
        }
    }

    fn push_type(&mut self, ty: &str) {
        self.push_site(ResponseSite::Type { ty: ty.to_string() });
    }

    fn push_type_ref(&mut self, ty: TypeRef) {
        match format_type(&ty) {
            Some(ty) => self.push_type(&ty),
            None => {
                let message = format!("the response type `{ty}` cannot be documented");
                self.analysis.unresolved.push(message);
            }
        }
    }

    fn push_site(&mut self, site: ResponseSite) {
        if !self.analysis.sites.contains(&site) {
            self.analysis.sites.push(site);
        }
    }

    fn unresolved(&mut self, what: &str) {
        let message = format!("could not resolve {what}");
        if !self.analysis.unresolved.contains(&message) {
            self.analysis.unresolved.push(message);
        }
    }
}

fn contains_impl_trait(ty: &Type) -> bool {
    match ty {
        Type::ImplTrait(_) => true,
        Type::Array(array) => contains_impl_trait(&array.elem),
        Type::Group(group) => contains_impl_trait(&group.elem),
        Type::Paren(paren) => contains_impl_trait(&paren.elem),
        Type::Ptr(pointer) => contains_impl_trait(&pointer.elem),
        Type::Reference(reference) => contains_impl_trait(&reference.elem),
        Type::Slice(slice) => contains_impl_trait(&slice.elem),
        Type::Tuple(tuple) => tuple.elems.iter().any(contains_impl_trait),
        Type::Path(path) => path.path.segments.iter().any(|segment| {
            if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                arguments.args.iter().any(|argument| match argument {
                    syn::GenericArgument::Type(ty) => contains_impl_trait(ty),
                    _ => false,
                })
            } else {
                false
            }
        }),
        _ => false,
    }
}

fn is_constructor(name: &str) -> bool {
    matches!(
        name,
        "Json"
            | "Html"
            | "Created"
            | "NoContent"
            | "Attachment"
            | "File"
            | "Sse"
            | "Redirect"
            | "Problem"
            | "Bytes"
            | "Body"
    )
}

fn is_status_path(path: &syn::Path) -> bool {
    path.segments
        .iter()
        .any(|segment| segment.ident == "StatusCode")
}

fn is_status_expression(expression: &Expr) -> bool {
    match expression {
        Expr::Path(path) => is_status_path(&path.path),
        Expr::Call(call) => match call.func.as_ref() {
            Expr::Path(path) => is_status_path(&path.path),
            _ => false,
        },
        Expr::Paren(paren) => is_status_expression(&paren.expr),
        Expr::Group(group) => is_status_expression(&group.expr),
        _ => false,
    }
}

fn callee_arity(function: &ItemFn) -> usize {
    function.sig.inputs.len()
}

/// Format a resolved type as a path the handler's module can name.
fn format_type(ty: &TypeRef) -> Option<String> {
    if ty.name == "Response" {
        return None;
    }
    if ty.name == "Unknown" || ty.args.iter().any(|arg| arg.name == "Unknown") {
        return None;
    }
    match ty.name.as_str() {
        // Fully qualify the std/axum names the analyzer resolves from a constructor.
        "String" => Some("::std::string::String".to_string()),
        "str" => Some("&'static str".to_string()),
        "Vec" if ty.args.len() == 1 && ty.args[0].name == "u8" => {
            Some("::std::vec::Vec<u8>".to_string())
        }
        _ => Some(ty.to_string()),
    }
}

fn dedupe(sites: &mut Vec<ResponseSite>) {
    let mut deduped = Vec::with_capacity(sites.len());
    for site in sites.drain(..) {
        if !deduped.contains(&site) {
            deduped.push(site);
        }
    }
    *sites = deduped;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::semantic::Index;

    fn analyze(source: &str) -> Analysis {
        let file = syn::parse_file(source).expect("valid source");
        let mut index = Index::default();
        index.insert_items(&file.items);
        let function = index.handlers().first().expect("one handler");
        analyze_handler(function, &index)
    }

    #[test]
    fn direct_json_is_resolved() {
        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            #[handler]
            async fn show() -> impl IntoResponse { Json(Note { id: 1 }) }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::Type {
                ty: "Json<Note>".to_string()
            }]
        );
    }

    #[test]
    fn local_bindings_are_resolved() {
        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            #[handler]
            async fn show() -> impl IntoResponse {
                let note = Note { id: 1 };
                Json(note)
            }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::Type {
                ty: "Json<Note>".to_string()
            }]
        );
    }

    #[test]
    fn helper_calls_use_a_witness() {
        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            fn load() -> Json<Note> { Json(Note { id: 1 }) }
            #[handler]
            async fn show() -> impl IntoResponse { load() }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::Call {
                path: "load".to_string(),
                arity: 0,
                is_async: false,
            }]
        );
    }

    #[test]
    fn into_response_is_opaque() {
        let analysis = analyze(
            r#"
            #[handler]
            async fn show() -> impl IntoResponse {
                (StatusCode::CREATED, "x").into_response()
            }
            "#,
        );
        assert!(analysis.sites.is_empty());
        assert!(!analysis.unresolved.is_empty());
    }

    #[test]
    fn into_documented_response_documents_the_receiver() {
        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            #[handler]
            async fn show() -> impl IntoResponse {
                Json(Note { id: 1 }).into_documented_response()
            }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::Type {
                ty: "Json<Note>".to_string()
            }]
        );
    }

    #[test]
    fn text_and_html_are_resolved() {
        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            #[handler]
            async fn text() -> impl IntoResponse { "ok" }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::Type {
                ty: "&'static str".to_string()
            }]
        );

        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            #[handler]
            async fn html() -> impl IntoResponse { Html(Note { id: 1 }) }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::Type {
                ty: "Html<Note>".to_string()
            }]
        );
    }

    #[test]
    fn method_calls_use_a_witness() {
        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            struct App;
            impl App {
                fn load(&self) -> Json<Note> { Json(Note { id: 1 }) }
            }
            #[handler]
            async fn show(app: App) -> impl IntoResponse { app.load() }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::Call {
                path: "App::load".to_string(),
                arity: 1,
                is_async: false,
            }]
        );
    }

    #[test]
    fn a_helper_returning_response_is_opaque() {
        let analysis = analyze(
            r#"
            fn make() -> Response { todo!() }
            #[handler]
            async fn show() -> impl IntoResponse { make() }
            "#,
        );
        assert!(analysis.sites.is_empty());
        assert!(!analysis.unresolved.is_empty());
    }

    #[test]
    fn status_tuple_keeps_the_body() {
        let analysis = analyze(
            r#"
            struct Note { id: u64 }
            #[handler]
            async fn show() -> impl IntoResponse {
                (StatusCode::CREATED, Json(Note { id: 1 }))
            }
            "#,
        );
        assert_eq!(
            analysis.sites,
            vec![ResponseSite::StatusTuple {
                inner: "Json<Note>".to_string()
            }]
        );
    }
}
