//! Generate the documentation slice the contract harness compiles.
//!
//! The harness must not depend on the application package: a bin-only package has no library
//! target to depend on, and even a library package would otherwise be compiled twice. This module
//! copies the documentation surface of the application into a generated crate under
//! `target/roundhouse/slice` and leaves the rest of the source behind.
//!
//! The slice keeps source text verbatim. `#[handler]` keys its analyzer index by a hash of the
//! handler's signature and body tokens, so re-emitting items through `quote!` would change the
//! key. Items that are not needed are blanked instead, and stubs are padded back to the original
//! line count so `file!()`/`line!()` in `routes!` keep producing the committed source locations.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{LineColumn, TokenStream, TokenTree};
use quote::{ToTokens, quote};
use serde_json::Value;
use syn::parse::Parser;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, FnArg, GenericParam, Item, ReturnType, Type, UseTree};

use crate::dependency::{Dependency, RoundhouseDependency};

/// The marker file written into the generated package. The macros switch to documentation-only
/// expansion when they see it in `CARGO_MANIFEST_DIR`.
pub const MARKER: &str = ".roundhouse-slice";

/// Everything the slice generator needs from the application package.
pub struct Input<'a> {
    pub root: &'a Path,
    pub entry: &'a Path,
    pub package_name: &'a str,
    pub version: &'a str,
    pub edition: &'a str,
    pub crate_name: &'a str,
    pub index_path: Option<&'a Path>,
    pub roundhouse: &'a RoundhouseDependency,
    pub dependencies: &'a [Dependency],
    pub features: &'a [String],
    pub all_features: &'a [String],
}

/// The files of the generated package, keyed by their path relative to its root.
pub struct Output {
    pub files: SliceFiles,
}

/// The generated files, keyed by their path relative to the package root.
pub type SliceFiles = Vec<(PathBuf, Vec<u8>)>;

pub fn generate(
    input: &Input<'_>,
    extra_dependencies: &BTreeSet<String>,
) -> Result<Output, Box<dyn Error>> {
    let tree = Tree::load(input.root, input.entry, input.features)?;
    let plan = plan(&tree, input)?;
    render(&tree, &plan, input, extra_dependencies)
}

// -------------------------------------------------------------------------------------------------
// Source tree
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Struct,
    Enum,
    Union,
    Alias,
    Const,
    Static,
    Fn,
    Impl,
    Trait,
    Use,
    MacroDef,
    MacroCall,
    Mod,
    InlineMod,
    Other,
}

struct FileData {
    abs: PathBuf,
    rel: PathBuf,
    source: String,
    parsed: syn::File,
}

struct ModuleData {
    path: Vec<String>,
    file: usize,
    items: Vec<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
    inline: bool,
}

struct ItemData {
    module: usize,
    file: usize,
    start: usize,
    end: usize,
    start_line: usize,
    end_line: usize,
    item: Item,
    kind: Kind,
    name: Option<String>,
    child_module: Option<usize>,
}

struct Tree {
    root: PathBuf,
    features: HashSet<String>,
    files: Vec<FileData>,
    modules: Vec<ModuleData>,
    items: Vec<ItemData>,
    root_module: usize,
    file_index: HashMap<PathBuf, usize>,
}

impl Tree {
    fn load(root: &Path, entry: &Path, features: &[String]) -> Result<Self, Box<dyn Error>> {
        let root = root.canonicalize()?;
        let entry = entry.canonicalize()?;
        let mut tree = Self {
            root,
            features: features.iter().cloned().collect(),
            files: Vec::new(),
            modules: Vec::new(),
            items: Vec::new(),
            root_module: 0,
            file_index: HashMap::new(),
        };
        tree.root_module = tree.load_module(&entry, vec!["crate".into()], None, true)?;
        Ok(tree)
    }

    fn load_module(
        &mut self,
        abs: &Path,
        path: Vec<String>,
        parent: Option<usize>,
        inline: bool,
    ) -> Result<usize, Box<dyn Error>> {
        let abs = abs.canonicalize()?;
        let file = self.load_file(&abs)?;
        let module = self.modules.len();
        self.modules.push(ModuleData {
            path,
            file,
            items: Vec::new(),
            parent,
            children: Vec::new(),
            inline,
        });
        if let Some(parent) = parent {
            self.modules[parent].children.push(module);
        }

        let items = self.files[file].parsed.items.clone();
        for item in items {
            let (start_line, end_line) = (item.span().start().line, item.span().end().line);
            let start = offset(&self.files[file].source, item.span().start());
            let end = offset(&self.files[file].source, item.span().end());
            let (kind, name) = classify_item(&item);
            let mut child_module = None;

            if let Item::Mod(module_item) = &item
                && cfg_enabled(&module_item.attrs, &self.features)
            {
                match &module_item.content {
                    Some((_, content)) => {
                        let child_path = {
                            let mut child_path = self.modules[module].path.clone();
                            child_path.push(module_item.ident.to_string());
                            child_path
                        };
                        let child = self.load_inline_module(file, &child_path, module, content)?;
                        child_module = Some(child);
                    }
                    None => {
                        let child_abs = resolve_child(abs.as_path(), module_item)?;
                        let mut child_path = self.modules[module].path.clone();
                        child_path.push(module_item.ident.to_string());
                        let child =
                            self.load_module(&child_abs, child_path, Some(module), false)?;
                        child_module = Some(child);
                    }
                }
            }

            let index = self.items.len();
            self.items.push(ItemData {
                module,
                file,
                start,
                end,
                start_line,
                end_line,
                item,
                kind,
                name,
                child_module,
            });
            self.modules[module].items.push(index);
        }

        Ok(module)
    }

    fn load_inline_module(
        &mut self,
        file: usize,
        path: &[String],
        parent: usize,
        content: &[Item],
    ) -> Result<usize, Box<dyn Error>> {
        let module = self.modules.len();
        self.modules.push(ModuleData {
            path: path.to_vec(),
            file,
            items: Vec::new(),
            parent: Some(parent),
            children: Vec::new(),
            inline: true,
        });
        self.modules[parent].children.push(module);

        for item in content {
            let start = offset(&self.files[file].source, item.span().start());
            let end = offset(&self.files[file].source, item.span().end());
            let (kind, name) = classify_item(item);
            let mut child_module = None;
            if let Item::Mod(module_item) = item
                && cfg_enabled(&module_item.attrs, &self.features)
            {
                match &module_item.content {
                    Some((_, nested)) => {
                        let mut child_path = path.to_vec();
                        child_path.push(module_item.ident.to_string());
                        let child = self.load_inline_module(file, &child_path, module, nested)?;
                        child_module = Some(child);
                    }
                    None => {
                        let abs = self.files[file].abs.clone();
                        let child_abs = resolve_child(abs.as_path(), module_item)?;
                        let mut child_path = path.to_vec();
                        child_path.push(module_item.ident.to_string());
                        let child =
                            self.load_module(&child_abs, child_path, Some(module), false)?;
                        child_module = Some(child);
                    }
                }
            }
            let index = self.items.len();
            self.items.push(ItemData {
                module,
                file,
                start,
                end,
                start_line: item.span().start().line,
                end_line: item.span().end().line,
                item: item.clone(),
                kind,
                name,
                child_module,
            });
            self.modules[module].items.push(index);
        }

        Ok(module)
    }

    fn load_file(&mut self, abs: &Path) -> Result<usize, Box<dyn Error>> {
        if let Some(index) = self.file_index.get(abs) {
            return Ok(*index);
        }
        let source = fs::read_to_string(abs)?;
        let parsed = syn::parse_file(&source)
            .map_err(|error| format!("could not parse {}: {error}", abs.display()))?;
        let rel = abs
            .strip_prefix(&self.root)
            .map_err(|_| {
                format!(
                    "{} is outside the package root {}; the documentation slice cannot mirror it",
                    abs.display(),
                    self.root.display()
                )
            })?
            .to_path_buf();
        let index = self.files.len();
        self.files.push(FileData {
            abs: abs.to_path_buf(),
            rel,
            source,
            parsed,
        });
        self.file_index.insert(abs.to_path_buf(), index);
        Ok(index)
    }

    fn resolve(&self, module: usize, name: &str) -> Result<Option<usize>, Box<dyn Error>> {
        let name = self.alias(module, name).unwrap_or_else(|| name.to_string());
        let mut candidates: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.name.as_deref() == Some(name.as_str()))
            .filter(|(_, item)| {
                matches!(
                    item.kind,
                    Kind::Struct
                        | Kind::Enum
                        | Kind::Union
                        | Kind::Alias
                        | Kind::Fn
                        | Kind::Const
                        | Kind::Static
                        | Kind::Trait
                        | Kind::MacroDef
                )
            })
            .map(|(index, _)| index)
            .collect();
        if candidates.len() <= 1 {
            return Ok(candidates.pop());
        }
        // Prefer an item declared in the referring module, then an ancestor module, then a
        // sibling with the longest shared module-path prefix.
        let referring = &self.modules[module].path;
        candidates.sort_by_key(|index| {
            let path = &self.modules[self.items[*index].module].path;
            let shared = path
                .iter()
                .zip(referring.iter())
                .take_while(|(left, right)| left == right)
                .count();
            (
                std::cmp::Reverse(shared),
                path.len(),
                self.items[*index].start,
            )
        });
        Ok(candidates.first().copied())
    }

    fn alias(&self, module: usize, name: &str) -> Option<String> {
        for item in &self.modules[module].items {
            let ItemData {
                item: Item::Use(item),
                ..
            } = &self.items[*item]
            else {
                continue;
            };
            let mut bindings = Vec::new();
            collect_use_bindings(&item.tree, &mut Vec::new(), &mut bindings);
            for (bound, path) in bindings {
                if bound == name {
                    return path.last().cloned();
                }
            }
        }
        None
    }

    fn impls_for(&self, name: &str) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == Kind::Impl && item.name.as_deref() == Some(name))
            .map(|(index, _)| index)
            .collect()
    }
}

fn classify_item(item: &Item) -> (Kind, Option<String>) {
    match item {
        Item::Struct(item) => (Kind::Struct, Some(item.ident.to_string())),
        Item::Enum(item) => (Kind::Enum, Some(item.ident.to_string())),
        Item::Union(item) => (Kind::Union, Some(item.ident.to_string())),
        Item::Type(item) => (Kind::Alias, Some(item.ident.to_string())),
        Item::Const(item) => (Kind::Const, Some(item.ident.to_string())),
        Item::Static(item) => (Kind::Static, Some(item.ident.to_string())),
        Item::Fn(item) => (Kind::Fn, Some(item.sig.ident.to_string())),
        Item::Impl(item) => (Kind::Impl, self_type_name(&item.self_ty)),
        Item::Trait(item) => (Kind::Trait, Some(item.ident.to_string())),
        Item::Use(_) => (Kind::Use, None),
        Item::Macro(item) if item.ident.is_some() => {
            (Kind::MacroDef, item.ident.as_ref().map(ToString::to_string))
        }
        Item::Macro(item) => (
            Kind::MacroCall,
            item.mac
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
        ),
        Item::Mod(item) if item.content.is_some() => {
            (Kind::InlineMod, Some(item.ident.to_string()))
        }
        Item::Mod(item) => (Kind::Mod, Some(item.ident.to_string())),
        _ => (Kind::Other, None),
    }
}

fn self_type_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn resolve_child(current: &Path, module: &syn::ItemMod) -> Result<PathBuf, Box<dyn Error>> {
    if let Some(path) = path_attribute(&module.attrs) {
        let base = current
            .parent()
            .ok_or_else(|| format!("{} has no parent directory", current.display()))?;
        let candidate = base.join(&path);
        if candidate.is_file() {
            return Ok(candidate);
        }
        return Err(format!(
            "`#[path = \"{path}\"] mod {};` in {} does not resolve to {}",
            module.ident,
            current.display(),
            candidate.display()
        )
        .into());
    }

    let directory = child_directory(current);
    let direct = directory.join(format!("{}.rs", module.ident));
    if direct.is_file() {
        return Ok(direct);
    }
    let nested = directory.join(module.ident.to_string()).join("mod.rs");
    if nested.is_file() {
        return Ok(nested);
    }
    Err(format!(
        "could not find the file for `mod {};` in {} (looked for {} and {}); \
         move the module out of a non-default `#[path]`, or use the linked harness mode",
        module.ident,
        current.display(),
        direct.display(),
        nested.display()
    )
    .into())
}

fn child_directory(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or_else(|| Path::new("."));
    match file.file_stem().and_then(|stem| stem.to_str()) {
        Some("lib" | "main" | "mod") => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
        None => parent.to_path_buf(),
    }
}

fn path_attribute(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("path")
            && let syn::Meta::NameValue(value) = &attr.meta
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(literal),
                ..
            }) = &value.value
        {
            return Some(literal.value());
        }
    }
    None
}

fn cfg_enabled(attrs: &[Attribute], features: &HashSet<String>) -> bool {
    for attr in attrs {
        if !attr.path().is_ident("cfg") {
            continue;
        }
        let Ok(meta) = attr.parse_args::<syn::Meta>() else {
            return true;
        };
        if !eval_cfg(&meta, features) {
            return false;
        }
    }
    true
}

fn eval_cfg(meta: &syn::Meta, features: &HashSet<String>) -> bool {
    match meta {
        syn::Meta::Path(path) => match path.get_ident().map(|ident| ident.to_string()).as_deref() {
            Some("test" | "doctest") => false,
            Some("unix") => cfg!(unix),
            Some("windows") => cfg!(windows),
            // Unknown predicates are kept: the slice enables the same feature set, and the
            // compiler evaluates the copy again.
            _ => true,
        },
        syn::Meta::NameValue(value) => {
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(literal),
                ..
            }) = &value.value
            else {
                return true;
            };
            let literal_value = literal.value();
            if value.path.is_ident("feature") {
                return features.contains(&literal_value);
            }
            let target = |name: &str, fallback: &str| {
                std::env::var(name).unwrap_or_else(|_| fallback.to_string())
            };
            match value
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .as_deref()
            {
                Some("target_os") => {
                    target("CARGO_CFG_TARGET_OS", std::env::consts::OS) == literal_value
                }
                Some("target_arch") => {
                    target("CARGO_CFG_TARGET_ARCH", std::env::consts::ARCH) == literal_value
                }
                Some("target_family") => {
                    target("CARGO_CFG_TARGET_FAMILY", std::env::consts::FAMILY) == literal_value
                }
                Some("target_env") => target("CARGO_CFG_TARGET_ENV", "") == literal_value,
                Some("target_vendor") => target("CARGO_CFG_TARGET_VENDOR", "") == literal_value,
                _ => true,
            }
        }
        syn::Meta::List(list) => {
            let Some(ident) = list.path.get_ident() else {
                return true;
            };
            match ident.to_string().as_str() {
                "not" => {
                    let parser =
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
                    match parser.parse2(list.tokens.clone()) {
                        Ok(items) => items
                            .first()
                            .map(|item| !eval_cfg(item, features))
                            .unwrap_or(true),
                        Err(_) => true,
                    }
                }
                "all" | "any" => {
                    let parser =
                        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated;
                    let Ok(items) = parser.parse2(list.tokens.clone()) else {
                        return true;
                    };
                    if ident == "all" {
                        items.iter().all(|item| eval_cfg(item, features))
                    } else {
                        items.iter().any(|item| eval_cfg(item, features))
                    }
                }
                _ => true,
            }
        }
    }
}

fn offset(source: &str, position: LineColumn) -> usize {
    if position.line == 0 {
        return 0;
    }
    let mut offset = 0;
    for (index, line) in source.split('\n').enumerate() {
        if index + 1 == position.line {
            return offset
                + line
                    .char_indices()
                    .nth(position.column)
                    .map(|(byte, _)| byte)
                    .unwrap_or(line.len());
        }
        offset += line.len() + 1;
    }
    source.len()
}

// -------------------------------------------------------------------------------------------------
// Planning
// -------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Class {
    Stub,
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Decision {
    Drop,
    Stub,
    Keep,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Body,
    Params,
    Response,
    Error,
    State,
    Auth,
    Extractor,
    Unknown,
}

struct Plan {
    decisions: HashMap<usize, Decision>,
    module_keep: Vec<bool>,
    used_deps: BTreeSet<String>,
    extra_files: Vec<(PathBuf, Vec<u8>)>,
}

struct Planner<'a> {
    tree: &'a Tree,
    input: &'a Input<'a>,
    decisions: HashMap<usize, Decision>,
    classes: HashMap<usize, Class>,
    queue: Vec<(usize, Class)>,
    impls_done: HashSet<usize>,
    refs: Refs,
    used_deps: BTreeSet<String>,
}

fn plan(tree: &Tree, input: &Input<'_>) -> Result<Plan, Box<dyn Error>> {
    let mut planner = Planner {
        tree,
        input,
        decisions: HashMap::new(),
        classes: HashMap::new(),
        queue: Vec::new(),
        impls_done: HashSet::new(),
        refs: Refs::default(),
        used_deps: BTreeSet::new(),
    };
    planner.seed()?;
    planner.close()?;
    planner.imports()?;
    planner.close()?;
    let extra_files = planner.support_files()?;
    let module_keep = planner.module_keep();
    planner.finalize_modules(&module_keep);
    Ok(Plan {
        decisions: planner.decisions,
        module_keep,
        used_deps: planner.used_deps,
        extra_files,
    })
}

impl<'a> Planner<'a> {
    fn seed(&mut self) -> Result<(), Box<dyn Error>> {
        for index in 0..self.tree.items.len() {
            let item = &self.tree.items[index];
            match item.kind {
                Kind::MacroCall
                    if matches!(item.name.as_deref(), Some("api" | "app_state" | "routes")) =>
                {
                    self.keep(index);
                    if item.name.as_deref() == Some("api") {
                        self.api_types(&item.item)?;
                    }
                }
                Kind::MacroDef | Kind::Trait => {
                    // Kept only when an included item needs them; the import pass and the
                    // reference pass add them.
                }
                Kind::Fn if has_handler(&item.item) => {
                    self.keep(index);
                    self.handler_types(&item.item)?;
                }
                _ => {}
            }
        }

        if let Some(index_path) = self.input.index_path {
            self.index_types(index_path)?;
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), Box<dyn Error>> {
        loop {
            let mut changed = false;
            while let Some((index, class)) = self.queue.pop() {
                changed = true;
                self.process_type(index, class)?;
            }
            self.add_doc_impls(&mut changed)?;
            self.add_referenced_items(&mut changed)?;
            if !changed {
                break;
            }
        }
        Ok(())
    }

    fn process_type(&mut self, index: usize, class: Class) -> Result<(), Box<dyn Error>> {
        let decision = match class {
            Class::Stub => {
                if stub_text(&self.tree.items[index].item).is_some() {
                    Decision::Stub
                } else {
                    Decision::Keep
                }
            }
            Class::Full => Decision::Keep,
        };
        let previous = self.decisions.insert(index, decision);
        let _ = previous;

        let item = self.tree.items[index].item.clone();
        match (&item, class) {
            (Item::Struct(item), Class::Full) => {
                for field in &item.fields {
                    self.classify(&field.ty, Role::Body, &generic_names(&item.generics))?;
                }
            }
            (Item::Enum(item), Class::Full) => {
                let generics = generic_names(&item.generics);
                for variant in &item.variants {
                    for field in &variant.fields {
                        self.classify(&field.ty, Role::Body, &generics)?;
                    }
                }
            }
            (Item::Union(item), Class::Full) => {
                let generics = generic_names(&item.generics);
                for field in &item.fields.named {
                    self.classify(&field.ty, Role::Body, &generics)?;
                }
            }
            (Item::Type(item), _) => {
                self.classify(&item.ty, Role::Body, &generic_names(&item.generics))?;
            }
            _ => {}
        }

        // Attributes can name derive crates and helper macros. Record their roots and keep any
        // local macro they mention.
        self.collect_attribute_refs(&item, index);
        Ok(())
    }

    fn handler_types(&mut self, item: &Item) -> Result<(), Box<dyn Error>> {
        let Item::Fn(function) = item else {
            return Ok(());
        };
        let generics = generic_names(&function.sig.generics);
        for argument in &function.sig.inputs {
            let FnArg::Typed(argument) = argument else {
                continue;
            };
            self.classify(&argument.ty, Role::Extractor, &generics)?;
        }
        if let ReturnType::Type(_, ty) = &function.sig.output {
            self.classify(ty, Role::Response, &generics)?;
        }
        Ok(())
    }

    fn classify(
        &mut self,
        ty: &Type,
        role: Role,
        generics: &HashSet<String>,
    ) -> Result<(), Box<dyn Error>> {
        match ty {
            Type::Reference(reference) => self.classify(&reference.elem, role, generics),
            Type::Paren(paren) => self.classify(&paren.elem, role, generics),
            Type::Group(group) => self.classify(&group.elem, role, generics),
            Type::Slice(slice) => self.classify(&slice.elem, role, generics),
            Type::Array(array) => self.classify(&array.elem, role, generics),
            Type::Ptr(pointer) => self.classify(&pointer.elem, role, generics),
            Type::Tuple(tuple) => {
                for element in &tuple.elems {
                    self.classify(element, role, generics)?;
                }
                Ok(())
            }
            Type::ImplTrait(value) => {
                for bound in &value.bounds {
                    if let syn::TypeParamBound::Trait(bound) = bound {
                        self.local_trait(&bound.path, generics);
                    }
                }
                Ok(())
            }
            Type::TraitObject(value) => {
                for bound in &value.bounds {
                    if let syn::TypeParamBound::Trait(bound) = bound {
                        self.local_trait(&bound.path, generics);
                    }
                }
                Ok(())
            }
            Type::BareFn(function) => {
                for input in &function.inputs {
                    self.classify(&input.ty, role, generics)?;
                }
                if let ReturnType::Type(_, output) = &function.output {
                    self.classify(output, role, generics)?;
                }
                Ok(())
            }
            Type::Path(path) => self.classify_path(&path.path, role, generics),
            _ => Ok(()),
        }
    }

    fn classify_path(
        &mut self,
        path: &syn::Path,
        role: Role,
        generics: &HashSet<String>,
    ) -> Result<(), Box<dyn Error>> {
        let Some(segment) = path.segments.last() else {
            return Ok(());
        };
        let name = segment.ident.to_string();
        let arguments: Vec<&Type> = match &segment.arguments {
            syn::PathArguments::AngleBracketed(arguments) => arguments
                .args
                .iter()
                .filter_map(|argument| match argument {
                    syn::GenericArgument::Type(ty) => Some(ty),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        if !generics.contains(&name) {
            match name.as_str() {
                "Option" | "Vec" | "Box" | "Arc" | "Rc" | "Cow" | "Mapped" | "Strict"
                | "Either" | "And" | "WithRejection" => {
                    for argument in arguments {
                        self.classify(argument, role, generics)?;
                    }
                    return Ok(());
                }
                "Result" => {
                    if let Some(ok) = arguments.first() {
                        let inner = if role == Role::Extractor {
                            Role::Extractor
                        } else {
                            role
                        };
                        self.classify(ok, inner, generics)?;
                    }
                    if let Some(error) = arguments.get(1) {
                        self.classify(error, Role::Error, generics)?;
                    }
                    return Ok(());
                }
                "Json" | "Form" => {
                    for argument in arguments {
                        self.classify(argument, Role::Body, generics)?;
                    }
                    return Ok(());
                }
                "Path" | "Query" => {
                    for argument in arguments {
                        self.classify(argument, Role::Params, generics)?;
                    }
                    return Ok(());
                }
                "State" | "Extension" | "ConnectInfo" => {
                    for argument in arguments {
                        self.classify(argument, Role::State, generics)?;
                    }
                    return Ok(());
                }
                "Auth" => {
                    for argument in arguments {
                        self.classify(argument, Role::Auth, generics)?;
                    }
                    return Ok(());
                }
                "Status" | "Created" | "Accepted" | "NoContent" | "NotFound" | "Empty"
                | "Documented" | "Ok" => {
                    for argument in arguments {
                        self.classify(argument, Role::Response, generics)?;
                    }
                    return Ok(());
                }
                _ => {}
            }

            if let Some(index) = self.tree.resolve_for(self.current_module(), &name)? {
                let item = &self.tree.items[index];
                match item.kind {
                    Kind::Struct | Kind::Enum | Kind::Union | Kind::Alias => {
                        let class = self.class_for(index, role);
                        self.want(index, class)?;
                    }
                    Kind::Fn | Kind::Const | Kind::Static | Kind::MacroDef => {
                        self.keep(index);
                    }
                    Kind::Trait => {
                        self.keep(index);
                    }
                    _ => {}
                }
            }
        }

        for argument in arguments {
            let next = match role {
                Role::Body | Role::Params | Role::Response => Role::Body,
                other => other,
            };
            self.classify(argument, next, generics)?;
        }
        Ok(())
    }

    fn class_for(&self, index: usize, role: Role) -> Class {
        if schemaish(&self.tree.items[index].item) {
            return Class::Full;
        }
        match role {
            Role::Body | Role::Params => Class::Full,
            Role::Response
            | Role::Error
            | Role::State
            | Role::Auth
            | Role::Extractor
            | Role::Unknown => Class::Stub,
        }
    }

    fn want(&mut self, index: usize, class: Class) -> Result<(), Box<dyn Error>> {
        if self.tree.items[index].kind == Kind::Alias {
            self.keep(index);
            self.queue.push((index, class));
            return Ok(());
        }
        if matches!(
            self.tree.items[index].kind,
            Kind::Struct | Kind::Enum | Kind::Union | Kind::Alias
        ) && class == Class::Stub
            && stub_text(&self.tree.items[index].item).is_none()
        {
            // A generic type cannot be stubbed without preserving its parameters; copy it whole.
            self.classes.insert(index, Class::Full);
            self.queue.push((index, Class::Full));
            return Ok(());
        }
        let previous = self.classes.get(&index).copied();
        if previous.is_none_or(|previous| class > previous) {
            self.classes.insert(index, class);
            self.queue.push((index, class));
        }
        Ok(())
    }

    fn keep(&mut self, index: usize) {
        self.decisions.entry(index).or_insert(Decision::Keep);
    }

    fn add_doc_impls(&mut self, changed: &mut bool) -> Result<(), Box<dyn Error>> {
        let names: Vec<(usize, String)> = self
            .classes
            .keys()
            .filter_map(|index| {
                self.tree.items[*index]
                    .name
                    .clone()
                    .map(|name| (*index, name))
            })
            .collect();
        for (type_index, name) in names {
            for impl_index in self.tree.impls_for(&name) {
                if !self.impls_done.insert(impl_index) {
                    continue;
                }
                if doc_trait(&self.tree.items[impl_index].item).is_none() {
                    continue;
                }
                let item = self.tree.items[impl_index].item.clone();
                // A documentation impl that names a variant needs the real definition.
                if item.to_token_stream().to_string().contains("Self ::") {
                    self.classes.insert(type_index, Class::Full);
                    self.queue.push((type_index, Class::Full));
                }
                self.decisions.insert(impl_index, Decision::Keep);
                self.walk_refs(&item, self.tree.items[impl_index].module)?;
                *changed = true;
            }
        }
        Ok(())
    }

    fn add_referenced_items(&mut self, changed: &mut bool) -> Result<(), Box<dyn Error>> {
        let refs = std::mem::take(&mut self.refs);
        self.used_deps.extend(refs.roots.iter().cloned());
        let mut local: Vec<(String, usize)> = refs
            .idents
            .iter()
            .map(|name| (name.clone(), 0usize))
            .collect();
        local.sort();
        for (name, _) in local {
            let Some(index) = self.tree.resolve(0, &name)? else {
                continue;
            };
            let item = &self.tree.items[index];
            match item.kind {
                Kind::MacroDef | Kind::Fn | Kind::Const | Kind::Static | Kind::Trait
                    if self.decisions.get(&index).copied() != Some(Decision::Keep) =>
                {
                    self.decisions.insert(index, Decision::Keep);
                    *changed = true;
                    let item = item.item.clone();
                    self.walk_refs(&item, self.tree.items[index].module)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn walk_refs(&mut self, item: &Item, module: usize) -> Result<(), Box<dyn Error>> {
        let mut refs = Refs::default();
        refs.visit_item(item);
        self.merge_refs(refs, module)?;
        Ok(())
    }

    fn merge_refs(&mut self, refs: Refs, _module: usize) -> Result<(), Box<dyn Error>> {
        self.used_deps.extend(refs.roots.iter().cloned());
        for name in &refs.idents {
            let Some(index) = self.tree.resolve(0, name)? else {
                continue;
            };
            match self.tree.items[index].kind {
                Kind::Struct | Kind::Enum | Kind::Union | Kind::Alias => {
                    let class = self.class_for(index, Role::Unknown);
                    self.want(index, class)?;
                }
                Kind::Fn | Kind::Const | Kind::Static | Kind::Trait | Kind::MacroDef => {
                    self.keep(index);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn collect_attribute_refs(&mut self, item: &Item, _index: usize) {
        let mut refs = Refs::default();
        for attr in item_attrs(item) {
            refs.visit_attribute(attr);
        }
        self.used_deps.extend(refs.roots);
    }

    fn local_trait(&mut self, path: &syn::Path, generics: &HashSet<String>) {
        let Some(segment) = path.segments.last() else {
            return;
        };
        let name = segment.ident.to_string();
        if generics.contains(&name) {
            return;
        }
        if let Ok(Some(index)) = self.tree.resolve(0, &name)
            && self.tree.items[index].kind == Kind::Trait
        {
            self.keep(index);
        }
    }

    fn api_types(&mut self, item: &Item) -> Result<(), Box<dyn Error>> {
        let Item::Macro(item) = item else {
            return Ok(());
        };
        let tokens: Vec<TokenTree> = item.mac.tokens.clone().into_iter().collect();
        let mut index = 0;
        while index < tokens.len() {
            if let TokenTree::Ident(ident) = &tokens[index] {
                let name = ident.to_string();
                if name == "state"
                    && let Some(ty) = parse_after_equals(&tokens, index + 1)
                {
                    self.classify(&ty, Role::State, &HashSet::new())?;
                }
                if name == "register"
                    && let Some(TokenTree::Group(group)) = tokens.get(index + 1)
                    && group.delimiter() == proc_macro2::Delimiter::Bracket
                {
                    let parser =
                        syn::punctuated::Punctuated::<Type, syn::Token![,]>::parse_terminated;
                    if let Ok(types) = parser.parse2(group.stream()) {
                        for ty in types {
                            self.classify(&ty, Role::Body, &HashSet::new())?;
                        }
                    }
                }
            }
            index += 1;
        }
        Ok(())
    }

    fn index_types(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let source = fs::read_to_string(path)?;
        let value: Value = serde_json::from_str(&source)?;
        let Some(handlers) = value["handlers"].as_array() else {
            return Ok(());
        };
        for handler in handlers {
            let Some(responses) = handler["responses"].as_array() else {
                continue;
            };
            for response in responses {
                let kind = response["kind"].as_str().unwrap_or_default();
                let ty = match kind {
                    "type" | "status_type" => response["ty"].as_str(),
                    "status_tuple" => response["inner"].as_str(),
                    _ => None,
                };
                if let Some(ty) = ty
                    && let Ok(ty) = syn::parse_str::<Type>(ty)
                {
                    self.classify(&ty, Role::Response, &HashSet::new())?;
                }
                if kind == "call"
                    && let Some(path) = response["path"].as_str()
                    && let Some(name) = path.split("::").last()
                    && let Some(index) = self.tree.resolve(0, name)?
                {
                    self.keep(index);
                    let item = self.tree.items[index].item.clone();
                    self.walk_refs(&item, self.tree.items[index].module)?;
                }
            }
        }
        Ok(())
    }

    fn imports(&mut self) -> Result<(), Box<dyn Error>> {
        for module in 0..self.tree.modules.len() {
            let mut refs = Refs::default();
            for index in &self.tree.modules[module].items {
                let item = &self.tree.items[*index];
                if item.kind == Kind::Use
                    || !matches!(
                        self.decisions.get(index).copied(),
                        Some(Decision::Keep | Decision::Stub)
                    )
                {
                    continue;
                }
                if item.kind == Kind::Mod || item.kind == Kind::InlineMod {
                    continue;
                }
                if item.kind == Kind::Fn && has_handler(&item.item) {
                    if let Item::Fn(function) = &item.item {
                        let mut signature = Refs::default();
                        for argument in &function.sig.inputs {
                            if let FnArg::Typed(argument) = argument {
                                signature.visit_type(&argument.ty);
                            }
                        }
                        if let ReturnType::Type(_, ty) = &function.sig.output {
                            signature.visit_type(ty);
                        }
                        refs.idents.extend(signature.idents);
                        refs.roots.extend(signature.roots);
                    }
                    continue;
                }
                refs.visit_item(&item.item);
            }
            self.used_deps.extend(refs.roots.iter().cloned());

            let uses: Vec<usize> = self.tree.modules[module]
                .items
                .iter()
                .copied()
                .filter(|index| self.tree.items[*index].kind == Kind::Use)
                .collect();
            for index in uses {
                let Item::Use(use_item) = &self.tree.items[index].item else {
                    continue;
                };
                let mut bindings = Vec::new();
                collect_use_bindings(&use_item.tree, &mut Vec::new(), &mut bindings);
                let glob = bindings.iter().any(|(name, _)| name == "*");
                let wildcard = bindings.iter().any(|(name, _)| name == "_");
                let used =
                    glob || wildcard || bindings.iter().any(|(name, _)| refs.idents.contains(name));
                if used {
                    self.keep(index);
                    for (_, path) in &bindings {
                        if let Some(first) = path.first() {
                            self.used_deps.insert(first.clone());
                        }
                        self.import_target(module, path)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn import_target(&mut self, module: usize, path: &[String]) -> Result<(), Box<dyn Error>> {
        let Some(last) = path.last() else {
            return Ok(());
        };
        if last == "*" || last == "_" {
            return Ok(());
        }
        let root = path.first().map(String::as_str).unwrap_or_default();
        if !matches!(root, "crate" | "self" | "super") {
            return Ok(());
        }
        let Some(index) = self.tree.resolve(module, last)? else {
            return Ok(());
        };
        match self.tree.items[index].kind {
            Kind::Struct | Kind::Enum | Kind::Union | Kind::Alias => {
                let class = self.class_for(index, Role::Unknown);
                self.want(index, class)?;
            }
            Kind::Fn | Kind::Const | Kind::Static | Kind::Trait | Kind::MacroDef => {
                self.keep(index);
            }
            _ => {}
        }
        Ok(())
    }

    fn module_keep(&self) -> Vec<bool> {
        let mut keep = vec![false; self.tree.modules.len()];
        for module in (0..self.tree.modules.len()).rev() {
            let mut value = false;
            for index in &self.tree.modules[module].items {
                let item = &self.tree.items[*index];
                match item.kind {
                    Kind::Mod | Kind::InlineMod => {
                        if let Some(child) = item.child_module
                            && keep[child]
                        {
                            value = true;
                        }
                    }
                    _ => {
                        if matches!(
                            self.decisions.get(index).copied(),
                            Some(Decision::Keep | Decision::Stub)
                        ) {
                            value = true;
                        }
                    }
                }
            }
            keep[module] = value || module == self.tree.root_module;
        }
        keep
    }

    fn support_files(&self) -> Result<SliceFiles, Box<dyn Error>> {
        let mut files: SliceFiles = Vec::new();
        for (index, item) in self.tree.items.iter().enumerate() {
            if !matches!(self.decisions.get(&index).copied(), Some(Decision::Keep)) {
                continue;
            }
            let mut includes = Includes::default();
            if item.kind == Kind::Fn && has_handler(&item.item) {
                if let Item::Fn(function) = &item.item {
                    for input in &function.sig.inputs {
                        if let FnArg::Typed(input) = input {
                            includes.visit_type(&input.ty);
                        }
                    }
                    if let ReturnType::Type(_, output) = &function.sig.output {
                        includes.visit_type(output);
                    }
                    for attr in &function.attrs {
                        includes.visit_attribute(attr);
                    }
                }
            } else {
                includes.visit_item(&item.item);
            }
            if let Some(name) = includes.unsupported.first() {
                return Err(format!(
                    "{}:{} uses `{name}`, which the documentation slice cannot reproduce; \
                     set ROUNDHOUSE_HARNESS=linked for this package",
                    self.tree.files[item.file].abs.display(),
                    item.start_line,
                )
                .into());
            }
            for path in &includes.files {
                let file = &self.tree.files[item.file];
                let base = file.rel.parent().unwrap_or_else(|| Path::new(""));
                let relative = normalize_relative(&base.join(path));
                if relative.starts_with("..") {
                    return Err(format!(
                        "{}:{} includes `{path}`, which leaves the package root; \
                         set ROUNDHOUSE_HARNESS=linked for this package",
                        file.abs.display(),
                        item.start_line,
                    )
                    .into());
                }
                if files.iter().any(|(existing, _)| existing == &relative) {
                    continue;
                }
                let source = self.input.root.join(&relative);
                let bytes = fs::read(&source).map_err(|error| {
                    format!(
                        "{}:{} includes `{path}`, but {} could not be read: {error}",
                        file.abs.display(),
                        item.start_line,
                        source.display()
                    )
                })?;
                files.push((relative, bytes));
            }
        }
        Ok(files)
    }

    fn finalize_modules(&mut self, keep: &[bool]) {
        for module in 0..self.tree.modules.len() {
            let items = self.tree.modules[module].items.clone();
            for index in items {
                let (kind, child) = {
                    let item = &self.tree.items[index];
                    (item.kind, item.child_module)
                };
                if matches!(kind, Kind::Mod | Kind::InlineMod)
                    && let Some(child) = child
                {
                    let decision = if keep[child] {
                        Decision::Keep
                    } else {
                        Decision::Drop
                    };
                    self.decisions.insert(index, decision);
                }
            }
        }
    }

    fn current_module(&self) -> usize {
        self.tree.root_module
    }
}

impl Tree {
    fn resolve_for(&self, module: usize, name: &str) -> Result<Option<usize>, Box<dyn Error>> {
        self.resolve(module, name)
    }
}

fn has_handler(item: &Item) -> bool {
    item_attrs(item).iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "handler")
    })
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Struct(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Const(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        _ => &[],
    }
}

fn generic_names(generics: &syn::Generics) -> HashSet<String> {
    generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Type(param) => Some(param.ident.to_string()),
            GenericParam::Const(param) => Some(param.ident.to_string()),
            GenericParam::Lifetime(_) => None,
        })
        .collect()
}

fn schemaish(item: &Item) -> bool {
    for attr in item_attrs(item) {
        let last = attr
            .path()
            .segments
            .last()
            .map(|segment| segment.ident.to_string())
            .unwrap_or_default();
        if matches!(
            last.as_str(),
            "schema" | "PathParams" | "QueryParams" | "Multipart"
        ) {
            return true;
        }
        if last == "derive" {
            let parser = syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated;
            if let Ok(paths) = attr.parse_args_with(parser) {
                for path in paths {
                    let name = path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string())
                        .unwrap_or_default();
                    if matches!(
                        name.as_str(),
                        "ApiSchema" | "ToSchema" | "PathParams" | "QueryParams" | "Multipart"
                    ) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn doc_trait(item: &Item) -> Option<String> {
    let Item::Impl(item) = item else {
        return None;
    };
    let (_, path, _) = item.trait_.as_ref()?;
    let name = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())?;
    let doc = matches!(
        name.as_str(),
        "DocumentedResponse"
            | "DocumentedError"
            | "DocumentedIntoResponse"
            | "ExtractDoc"
            | "ExtractArg"
            | "ApiSchema"
            | "ToSchema"
            | "AuthScheme"
    );
    doc.then_some(name)
}

fn stub_text(item: &Item) -> Option<String> {
    match item {
        Item::Struct(item) if item.generics.params.is_empty() => {
            let vis = &item.vis;
            let ident = &item.ident;
            Some(quote!(#vis struct #ident;).to_string())
        }
        Item::Enum(item) if item.generics.params.is_empty() => {
            let vis = &item.vis;
            let ident = &item.ident;
            Some(quote!(#vis enum #ident {}).to_string())
        }
        Item::Union(item) if item.generics.params.is_empty() => {
            let vis = &item.vis;
            let ident = &item.ident;
            Some(quote!(#vis union #ident { __roundhouse_slice: () }).to_string())
        }
        _ => None,
    }
}

fn parse_after_equals(tokens: &[TokenTree], after: usize) -> Option<Type> {
    let mut start = after;
    while start < tokens.len() {
        match &tokens[start] {
            TokenTree::Punct(punct) if punct.as_char() == '=' => break,
            _ => start += 1,
        }
    }
    if start >= tokens.len() {
        return None;
    }
    start += 1;
    let mut depth = 0usize;
    let mut end = start;
    while end < tokens.len() {
        match &tokens[end] {
            TokenTree::Punct(punct) if punct.as_char() == '<' => depth += 1,
            TokenTree::Punct(punct) if punct.as_char() == '>' => depth = depth.saturating_sub(1),
            TokenTree::Punct(punct) if punct.as_char() == ',' && depth == 0 => break,
            _ => {}
        }
        end += 1;
    }
    let stream: TokenStream = tokens[start..end].iter().cloned().collect();
    syn::parse2(stream).ok()
}

// -------------------------------------------------------------------------------------------------
// Reference scanning
// -------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Refs {
    idents: HashSet<String>,
    roots: HashSet<String>,
}

impl<'ast> Visit<'ast> for Refs {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        if let Some(last) = segments.last() {
            self.idents.insert(last.clone());
        }
        if segments.len() > 1 {
            self.roots.insert(segments[0].clone());
        }
        visit::visit_path(self, path);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        if let Some(first) = attr.path().segments.first() {
            self.roots.insert(first.ident.to_string());
        }
        scan_tokens(&attr.meta.to_token_stream(), self);
        visit::visit_attribute(self, attr);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(first) = mac.path.segments.first() {
            self.roots.insert(first.ident.to_string());
        }
        scan_tokens(&mac.tokens, self);
        visit::visit_macro(self, mac);
    }
}

fn scan_tokens(stream: &TokenStream, refs: &mut Refs) {
    let mut previous: Option<String> = None;
    for tree in stream.clone() {
        match tree {
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                refs.idents.insert(name.clone());
                if let Some(previous) = previous.take()
                    && previous == "::"
                {
                    refs.roots.insert(name.clone());
                }
                previous = Some(name);
            }
            TokenTree::Punct(punct) => {
                previous = Some(punct.as_char().to_string());
            }
            TokenTree::Group(group) => {
                scan_tokens(&group.stream(), refs);
                previous = None;
            }
            TokenTree::Literal(_) => previous = None,
        }
    }
}

#[derive(Default)]
struct Includes {
    files: Vec<String>,
    unsupported: Vec<String>,
}

impl<'ast> Visit<'ast> for Includes {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let Some(segment) = mac.path.segments.last() else {
            return;
        };
        let name = segment.ident.to_string();
        if is_support_macro(&name) {
            record_support_macro(&name, &mac.tokens, self);
        }
        scan_nested_support_macros(&mac.tokens, self);
        visit::visit_macro(self, mac);
    }

    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        scan_nested_support_macros(&attr.meta.to_token_stream(), self);
        visit::visit_attribute(self, attr);
    }
}

fn is_support_macro(name: &str) -> bool {
    matches!(
        name,
        "include_str" | "include_bytes" | "include" | "env" | "option_env"
    )
}

fn record_support_macro(name: &str, tokens: &TokenStream, includes: &mut Includes) {
    let Ok(literal) = syn::parse2::<syn::LitStr>(tokens.clone()) else {
        if name == "include" {
            includes.unsupported.push("include!".to_string());
        }
        return;
    };
    match name {
        "include_str" | "include_bytes" => includes.files.push(literal.value()),
        "include" => includes.unsupported.push("include!".to_string()),
        "env" | "option_env" => {
            let value = literal.value();
            if matches!(
                value.as_str(),
                "CARGO_MANIFEST_DIR" | "CARGO_BIN_NAME" | "OUT_DIR"
            ) {
                includes.unsupported.push(format!("env!(\"{value}\")"));
            }
        }
        _ => {}
    }
}

fn scan_nested_support_macros(stream: &TokenStream, includes: &mut Includes) {
    let tokens: Vec<TokenTree> = stream.clone().into_iter().collect();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                if is_support_macro(&name)
                    && let Some(TokenTree::Punct(punct)) = tokens.get(index + 1)
                    && punct.as_char() == '!'
                    && let Some(TokenTree::Group(group)) = tokens.get(index + 2)
                {
                    record_support_macro(&name, &group.stream(), includes);
                    index += 3;
                    continue;
                }
            }
            TokenTree::Group(group) => scan_nested_support_macros(&group.stream(), includes),
            _ => {}
        }
        index += 1;
    }
}

fn normalize_relative(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn collect_use_bindings(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    bindings: &mut Vec<(String, Vec<String>)>,
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_bindings(&path.tree, prefix, bindings);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            path.push(name.ident.to_string());
            bindings.push((name.ident.to_string(), path));
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            path.push(rename.ident.to_string());
            bindings.push((rename.rename.to_string(), path));
        }
        UseTree::Glob(_) => {
            let mut path = prefix.clone();
            path.push("*".to_string());
            bindings.push(("*".to_string(), path));
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_bindings(item, prefix, bindings);
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Rendering
// -------------------------------------------------------------------------------------------------

struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

fn render(
    tree: &Tree,
    plan: &Plan,
    input: &Input<'_>,
    extra_dependencies: &BTreeSet<String>,
) -> Result<Output, Box<dyn Error>> {
    let mut files = Vec::new();
    let mut emitted: HashSet<usize> = HashSet::new();

    for module in 0..tree.modules.len() {
        if module != tree.root_module && !plan.module_keep[module] {
            continue;
        }
        let file = tree.modules[module].file;
        // A file shared by an inline module is rendered once, from its outermost kept module.
        if let Some(parent) = tree.modules[module].parent
            && tree.modules[parent].file == file
        {
            continue;
        }
        if !emitted.insert(file) {
            continue;
        }
        let source = &tree.files[file].source;
        let mut edits = Vec::new();
        collect_edits(tree, plan, module, &mut edits)?;
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
        let mut rendered = source.clone();
        let mut boundary = source.len();
        for edit in edits {
            if edit.end > boundary {
                return Err(format!(
                    "internal error: overlapping documentation-slice edits in {}",
                    tree.files[file].abs.display()
                )
                .into());
            }
            rendered.replace_range(edit.start..edit.end, &edit.replacement);
            boundary = edit.start;
        }
        files.push((tree.files[file].rel.clone(), rendered.into_bytes()));
    }

    files.push((PathBuf::from(MARKER), Vec::new()));
    files.push((
        PathBuf::from("bin/contract.rs"),
        bin_source(input).into_bytes(),
    ));
    files.push((
        PathBuf::from("Cargo.toml"),
        manifest(tree, plan, input, extra_dependencies)?.into_bytes(),
    ));
    for (relative, bytes) in &plan.extra_files {
        if files.iter().all(|(existing, _)| existing != relative) {
            files.push((relative.clone(), bytes.clone()));
        }
    }
    Ok(Output { files })
}

fn collect_edits(
    tree: &Tree,
    plan: &Plan,
    module: usize,
    edits: &mut Vec<Edit>,
) -> Result<(), Box<dyn Error>> {
    let source = &tree.files[tree.modules[module].file].source;
    for index in &tree.modules[module].items {
        let item = &tree.items[*index];
        let decision = plan.decisions.get(index).copied().unwrap_or(Decision::Drop);
        match item.kind {
            Kind::Mod | Kind::InlineMod => {
                if decision == Decision::Drop {
                    edits.push(Edit {
                        start: item.start,
                        end: item.end,
                        replacement: blank(&source[item.start..item.end]),
                    });
                } else if let Some(child) = item.child_module
                    && tree.modules[child].inline
                {
                    collect_edits(tree, plan, child, edits)?;
                }
            }
            _ => match decision {
                Decision::Keep => {}
                Decision::Drop => edits.push(Edit {
                    start: item.start,
                    end: item.end,
                    replacement: blank(&source[item.start..item.end]),
                }),
                Decision::Stub => {
                    let stub = stub_text(&item.item).ok_or_else(|| {
                        format!(
                            "internal error: {} was classified as a stub but has no stub form",
                            tree.items[*index].name.as_deref().unwrap_or("<item>")
                        )
                    })?;
                    let padding = "\n".repeat(item.end_line.saturating_sub(item.start_line));
                    edits.push(Edit {
                        start: item.start,
                        end: item.end,
                        replacement: format!("{stub}{padding}"),
                    });
                }
            },
        }
    }
    Ok(())
}

fn blank(source: &str) -> String {
    source
        .chars()
        .map(|character| match character {
            '\n' | '\r' => character,
            _ => ' ',
        })
        .collect()
}

fn bin_source(input: &Input<'_>) -> String {
    format!(
        "// @generated by roundhouse. Do not edit.\n\
         // The documentation slice for `{}`.\n\
         \n\
         #[rustfmt::skip]\n\
         fn main() -> Result<(), Box<dyn std::error::Error>> {{\n\
             let output = std::env::args()\n\
                 .nth(1)\n\
                 .unwrap_or_else(|| \"{}\".to_string());\n\
             let root = std::env::args().nth(2);\n\
             let contract = {}::__roundhouse_contract();\n\
             let built = contract.build()?;\n\
             built.write_to_with_root(\n\
                 output,\n\
                 root.as_deref().map(::std::path::Path::new),\n\
             )?;\n\
             Ok(())\n\
         }}\n",
        input.package_name,
        crate::ARTIFACT_DIR,
        input.crate_name,
    )
}

fn manifest(
    tree: &Tree,
    plan: &Plan,
    input: &Input<'_>,
    extra_dependencies: &BTreeSet<String>,
) -> Result<String, Box<dyn Error>> {
    let mut manifest = String::new();
    writeln!(manifest, "[package]")?;
    writeln!(manifest, "name = {:?}", input.package_name)?;
    writeln!(manifest, "version = {:?}", input.version)?;
    writeln!(manifest, "edition = {:?}", input.edition)?;
    writeln!(manifest, "publish = false")?;
    writeln!(manifest)?;
    writeln!(manifest, "[workspace]")?;
    writeln!(manifest)?;
    writeln!(manifest, "[lib]")?;
    writeln!(manifest, "name = {:?}", input.crate_name)?;
    writeln!(
        manifest,
        "path = {:?}",
        tree.files[tree.modules[tree.root_module].file]
            .rel
            .to_string_lossy()
            .replace('\\', "/")
    )?;
    writeln!(manifest)?;
    writeln!(manifest, "[[bin]]")?;
    writeln!(manifest, "name = \"contract\"")?;
    writeln!(manifest, "path = \"bin/contract.rs\"")?;
    writeln!(manifest)?;
    writeln!(manifest, "[lints.rust]")?;
    writeln!(manifest, "unused_imports = \"allow\"")?;
    writeln!(manifest, "dead_code = \"allow\"")?;
    writeln!(manifest, "unused_variables = \"allow\"")?;
    writeln!(manifest, "unexpected_cfgs = \"allow\"")?;

    if !input.all_features.is_empty() {
        writeln!(manifest)?;
        writeln!(manifest, "[features]")?;
        let active: Vec<String> = input
            .features
            .iter()
            .filter(|feature| feature.as_str() != "default")
            .map(|feature| format!("{feature:?}"))
            .collect();
        writeln!(manifest, "default = [{}]", active.join(", "))?;
        for feature in input.all_features {
            if feature != "default" {
                writeln!(manifest, "{feature} = []")?;
            }
        }
    }

    writeln!(manifest)?;
    writeln!(manifest, "[dependencies]")?;
    match &input.roundhouse.path {
        Some(path) => writeln!(
            manifest,
            "roundhouse = {{ path = {:?}, default-features = false, features = [\"build\"] }}",
            path.display().to_string()
        )?,
        None => writeln!(
            manifest,
            "roundhouse = {{ version = \"={}\", default-features = false, features = [\"build\"] }}",
            input.roundhouse.version
        )?,
    }

    for dependency in input.dependencies {
        let key = dependency.key();
        if key == "roundhouse"
            || !(plan.used_deps.contains(key) || extra_dependencies.contains(key))
        {
            continue;
        }
        if !matches!(dependency.kind.as_str(), "normal") {
            continue;
        }
        if dependency.target.is_some() && !extra_dependencies.contains(key) {
            // A target-specific dependency is only emitted after the compiler asks for it, which
            // means the retained code is for the target being built.
            continue;
        }
        let mut fields = Vec::new();
        if key != dependency.name {
            fields.push(format!("package = {:?}", dependency.name));
        }
        if let Some(path) = &dependency.path {
            fields.push(format!("path = {:?}", path.display().to_string()));
        } else if let Some(git) = &dependency.git {
            fields.push(format!("git = {git:?}"));
            if let Some(rev) = &dependency.rev {
                fields.push(format!("rev = {rev:?}"));
            }
        } else {
            fields.push(format!("version = {:?}", dependency.req));
        }
        if !dependency.default_features {
            fields.push("default-features = false".to_string());
        }
        if !dependency.features.is_empty() {
            let features = dependency
                .features
                .iter()
                .map(|feature| format!("{feature:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            fields.push(format!("features = [{features}]"));
        }
        writeln!(manifest, "{key} = {{ {} }}", fields.join(", "))?;
    }

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfg_predicates_use_the_active_features() {
        let features: HashSet<String> = ["docs".to_string()].into_iter().collect();
        let enabled: Item = syn::parse_quote!(
            #[cfg(feature = "docs")]
            mod a;
        );
        let disabled: Item = syn::parse_quote!(
            #[cfg(feature = "other")]
            mod b;
        );
        let negated: Item = syn::parse_quote!(
            #[cfg(not(feature = "other"))]
            mod c;
        );
        let test_only: Item = syn::parse_quote!(
            #[cfg(test)]
            mod tests;
        );
        assert!(cfg_enabled(item_attrs(&enabled), &features));
        assert!(!cfg_enabled(item_attrs(&disabled), &features));
        assert!(cfg_enabled(item_attrs(&negated), &features));
        assert!(!cfg_enabled(item_attrs(&test_only), &features));
    }

    #[test]
    fn offsets_are_character_based() {
        let source = "// ünïcode\n\npub struct A;\n";
        assert_eq!(offset(source, LineColumn { line: 3, column: 0 }), 14);
        assert_eq!(
            offset(
                source,
                LineColumn {
                    line: 3,
                    column: 12
                }
            ),
            26
        );
    }

    #[test]
    fn blanking_preserves_line_structure() {
        assert_eq!(
            blank("fn main() {\n    x();\n}\n"),
            "           \n        \n \n"
        );
    }

    #[test]
    fn stubs_keep_visibility() {
        let item: Item = syn::parse_quote!(
            pub struct MissingNote;
        );
        assert_eq!(
            stub_text(&item).as_deref(),
            Some("pub struct MissingNote ;")
        );
        let item: Item = syn::parse_quote!(
            pub(crate) enum Error {
                A,
            }
        );
        assert_eq!(
            stub_text(&item).as_deref(),
            Some("pub (crate) enum Error { }")
        );
        let item: Item = syn::parse_quote!(
            pub struct Generic<T>(T);
        );
        assert_eq!(stub_text(&item), None);
    }

    #[test]
    fn use_bindings_follow_groups_and_renames() {
        let item: syn::ItemUse = syn::parse_quote!(
            use crate::error::{MissingNote, Invalid as Bad, *};
        );
        let mut bindings = Vec::new();
        collect_use_bindings(&item.tree, &mut Vec::new(), &mut bindings);
        let names: Vec<&str> = bindings.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["MissingNote", "Bad", "*"]);
        assert_eq!(
            bindings[1].1,
            ["crate", "error", "Invalid"].map(ToString::to_string)
        );
    }

    #[test]
    fn api_state_is_read_from_the_macro_tokens() {
        let item: Item = syn::parse_quote!(
            api! { state = crate::state::AppState, title = "x", version = "1", root = routes::MODULE }
        );
        let Item::Macro(item) = item else {
            panic!("macro item");
        };
        let tokens: Vec<TokenTree> = item.mac.tokens.clone().into_iter().collect();
        let ty = parse_after_equals(&tokens, 1).expect("state type");
        assert_eq!(quote!(#ty).to_string(), "crate :: state :: AppState");
    }
}
