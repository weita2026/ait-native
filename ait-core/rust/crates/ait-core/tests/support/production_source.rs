use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{Attribute, Meta, Token};

#[derive(Clone, Copy, PartialEq)]
enum Condition {
    False,
    True,
    Unknown,
}

fn condition(meta: &Meta) -> Condition {
    if meta.path().is_ident("test") {
        return Condition::False;
    }
    let Meta::List(list) = meta else {
        return Condition::Unknown;
    };
    let Ok(arguments) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
    else {
        return Condition::Unknown;
    };
    let values = arguments.iter().map(condition).collect::<Vec<_>>();
    if list.path.is_ident("all") {
        if values.contains(&Condition::False) {
            Condition::False
        } else if values.iter().all(|value| *value == Condition::True) {
            Condition::True
        } else {
            Condition::Unknown
        }
    } else if list.path.is_ident("any") {
        if values.contains(&Condition::True) {
            Condition::True
        } else if values.iter().all(|value| *value == Condition::False) {
            Condition::False
        } else {
            Condition::Unknown
        }
    } else if list.path.is_ident("not") && values.len() == 1 {
        match values[0] {
            Condition::False => Condition::True,
            Condition::True => Condition::False,
            Condition::Unknown => Condition::Unknown,
        }
    } else {
        Condition::Unknown
    }
}

fn excludes_production(meta: &Meta) -> bool {
    if meta.path().is_ident("test") {
        return true;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    if list.path.is_ident("cfg") {
        return list
            .parse_args::<Meta>()
            .is_ok_and(|predicate| condition(&predicate) == Condition::False);
    }
    if list.path.is_ident("cfg_attr") {
        if let Ok(arguments) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        {
            let mut arguments = arguments.iter();
            return arguments
                .next()
                .is_some_and(|predicate| condition(predicate) == Condition::True)
                && arguments.any(excludes_production);
        }
    }
    false
}

fn test_only(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| excludes_production(&attribute.meta))
}

macro_rules! attributes {
    ($node:expr, $kind:ident, $($variant:ident),+ $(,)?) => {
        match $node {
            $(syn::$kind::$variant(node) => node.attrs.as_slice(),)+
            _ => &[],
        }
    };
}

#[derive(Default)]
struct TestOnlyRanges(Vec<Range<usize>>);

impl TestOnlyRanges {
    fn exclude(&mut self, attributes: &[Attribute], span: proc_macro2::Span) -> bool {
        if test_only(attributes) {
            self.0.push(span.byte_range());
            true
        } else {
            false
        }
    }
}

macro_rules! visit_attributed {
    ($method:ident, $kind:ident) => {
        fn $method(&mut self, node: &'ast syn::$kind) {
            if !self.exclude(&node.attrs, node.span()) {
                visit::$method(self, node);
            }
        }
    };
}

impl<'ast> Visit<'ast> for TestOnlyRanges {
    fn visit_item(&mut self, node: &'ast syn::Item) {
        let attrs = attributes!(
            node,
            Item,
            Const,
            Enum,
            ExternCrate,
            Fn,
            ForeignMod,
            Impl,
            Macro,
            Mod,
            Static,
            Struct,
            Trait,
            TraitAlias,
            Type,
            Union,
            Use
        );
        if !self.exclude(attrs, node.span()) {
            visit::visit_item(self, node);
        }
    }

    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        let attrs = attributes!(
            node, Expr, Array, Assign, Async, Await, Binary, Block, Break, Call, Cast, Closure,
            Const, Continue, Field, ForLoop, Group, If, Index, Infer, Let, Lit, Loop, Macro, Match,
            MethodCall, Paren, Path, Range, RawAddr, Reference, Repeat, Return, Struct, Try,
            TryBlock, Tuple, Unary, Unsafe, While, Yield
        );
        if !self.exclude(attrs, node.span()) {
            visit::visit_expr(self, node);
        }
    }

    fn visit_impl_item(&mut self, node: &'ast syn::ImplItem) {
        let attrs = attributes!(node, ImplItem, Const, Fn, Type, Macro);
        if !self.exclude(attrs, node.span()) {
            visit::visit_impl_item(self, node);
        }
    }

    fn visit_trait_item(&mut self, node: &'ast syn::TraitItem) {
        let attrs = attributes!(node, TraitItem, Const, Fn, Type, Macro);
        if !self.exclude(attrs, node.span()) {
            visit::visit_trait_item(self, node);
        }
    }

    fn visit_foreign_item(&mut self, node: &'ast syn::ForeignItem) {
        let attrs = attributes!(node, ForeignItem, Fn, Static, Type, Macro);
        if !self.exclude(attrs, node.span()) {
            visit::visit_foreign_item(self, node);
        }
    }

    visit_attributed!(visit_local, Local);
    visit_attributed!(visit_stmt_macro, StmtMacro);
    visit_attributed!(visit_arm, Arm);
    visit_attributed!(visit_field, Field);
    visit_attributed!(visit_variant, Variant);
}

fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    result
}

fn module_paths(attributes: &[Attribute]) -> (Vec<PathBuf>, bool) {
    fn collect(meta: &Meta, certain: bool, paths: &mut Vec<PathBuf>, replaces: &mut bool) {
        if let Meta::NameValue(value) = meta {
            if value.path.is_ident("path") {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(path),
                    ..
                }) = &value.value
                {
                    paths.push(PathBuf::from(path.value()));
                    *replaces |= certain;
                }
            }
        } else if let Meta::List(list) = meta {
            if list.path.is_ident("cfg_attr") {
                if let Ok(arguments) =
                    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                {
                    let mut arguments = arguments.iter();
                    if let Some(predicate) = arguments.next() {
                        let state = condition(predicate);
                        if state != Condition::False {
                            for nested in arguments {
                                collect(
                                    nested,
                                    certain && state == Condition::True,
                                    paths,
                                    replaces,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    let mut paths = Vec::new();
    let mut replaces = false;
    for attribute in attributes {
        collect(&attribute.meta, true, &mut paths, &mut replaces);
    }
    (paths, replaces)
}

struct ModuleReferences<'a> {
    file: &'a Path,
    directory: PathBuf,
    attribute_directory: PathBuf,
    excluded: &'a [Range<usize>],
    edges: Vec<(PathBuf, bool)>,
    includes: Vec<(PathBuf, bool)>,
    unresolved_include: bool,
}

impl ModuleReferences<'_> {
    fn gated(&self, span: proc_macro2::Span) -> bool {
        let span = span.byte_range();
        self.excluded
            .iter()
            .any(|range| range.start <= span.start && span.end <= range.end)
    }
}

impl<'ast> Visit<'ast> for ModuleReferences<'_> {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let gated = self.gated(module.span());
        let (paths, replaces) = module_paths(&module.attrs);
        if let Some((_, items)) = &module.content {
            let mut directories = paths
                .iter()
                .map(|path| self.attribute_directory.join(path))
                .collect::<Vec<_>>();
            if !replaces {
                directories.push(self.directory.join(module.ident.to_string()));
            }
            let directory = self.directory.clone();
            let attribute_directory = self.attribute_directory.clone();
            for nested in directories {
                self.directory = normalized(&nested);
                self.attribute_directory = self.directory.clone();
                for item in items {
                    self.visit_item(item);
                }
            }
            self.directory = directory;
            self.attribute_directory = attribute_directory;
        } else {
            for path in paths {
                self.edges
                    .push((normalized(&self.attribute_directory.join(path)), gated));
            }
            if !replaces {
                let module = self.directory.join(module.ident.to_string());
                self.edges.push((module.with_extension("rs"), gated));
                self.edges.push((module.join("mod.rs"), gated));
            }
        }
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        if node
            .path
            .segments
            .last()
            .is_some_and(|part| part.ident == "include")
        {
            // Only a literal argument supplies a path edge. Macro/expression
            // arguments remain unresolved regardless of their spelling or
            // apparent builtin identity; never expand or resolve user macros.
            let arguments = Punctuated::<syn::LitStr, Token![,]>::parse_terminated
                .parse2(node.tokens.clone())
                .ok();
            let path = arguments
                .as_ref()
                .filter(|arguments| arguments.len() == 1)
                .and_then(|arguments| arguments.first().map(syn::LitStr::value));
            let gated = self.gated(node.span());
            if let Some(path) = path {
                self.includes
                    .push((normalized(&self.file.parent().unwrap().join(path)), gated));
            } else if !gated {
                self.unresolved_include = true;
            }
        }
        visit::visit_macro(self, node);
    }
}

/// Names convey no test evidence. Exclude a non-root file only when every
/// known module/include reference is test-gated or comes from a proven test
/// file. Unknown/cyclic inputs and any possible production reference stay in
/// production scope. An unresolved include in possible production code can
/// reach any input, so test aliases alone cannot exclude files in that case.
pub fn test_only_files(
    sources: &BTreeMap<PathBuf, String>,
    roots: &BTreeSet<PathBuf>,
) -> Result<BTreeSet<PathBuf>, syn::Error> {
    let mut incoming: BTreeMap<PathBuf, Vec<(PathBuf, bool)>> = BTreeMap::new();
    let mut unresolved_includes = BTreeSet::new();
    for (path, source) in sources {
        let file = syn::parse_file(source)?;
        let mut ranges = TestOnlyRanges::default();
        if test_only(&file.attrs) {
            ranges.0.push(0..source.len());
        } else {
            ranges.visit_file(&file);
        }
        let parent = path.parent().unwrap();
        let directory =
            if roots.contains(path) || path.file_name().is_some_and(|name| name == "mod.rs") {
                parent.to_path_buf()
            } else {
                parent.join(path.file_stem().unwrap())
            };
        let mut references = ModuleReferences {
            file: path,
            directory,
            attribute_directory: parent.to_path_buf(),
            excluded: &ranges.0,
            edges: Vec::new(),
            includes: Vec::new(),
            unresolved_include: false,
        };
        references.visit_file(&file);
        if references.unresolved_include {
            unresolved_includes.insert(path.clone());
        }
        for (target, gated) in references.includes {
            if sources.contains_key(&target) {
                incoming
                    .entry(target)
                    .or_default()
                    .push((path.clone(), gated));
            } else if !gated {
                // Even a literal absolute/out-of-inventory path is not proof
                // that it cannot refer to one of the relative input paths.
                unresolved_includes.insert(path.clone());
            }
        }
        for (target, gated) in references.edges {
            if sources.contains_key(&target) {
                incoming
                    .entry(target)
                    .or_default()
                    .push((path.clone(), gated));
            }
        }
    }
    let mut excluded = BTreeSet::new();
    loop {
        let next = incoming
            .iter()
            .filter(|(path, references)| {
                !roots.contains(*path)
                    && !excluded.contains(*path)
                    && references
                        .iter()
                        .all(|(parent, gated)| *gated || excluded.contains(parent))
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        if next.is_empty() {
            break;
        }
        excluded.extend(next);
    }
    if unresolved_includes
        .iter()
        .any(|path| !excluded.contains(path))
    {
        return Ok(BTreeSet::new());
    }
    Ok(excluded)
}

/// Mask only syntax proven absent when `test` is false. Unknown feature/platform
/// predicates remain in scope so the gate covers production on every target.
/// Preserve byte offsets and newlines for the existing literal/line guards.
pub fn production_source(source: &str) -> Result<String, syn::Error> {
    let file = syn::parse_file(source)?;
    let mut ranges = TestOnlyRanges::default();
    if test_only(&file.attrs) {
        ranges.0.push(0..source.len());
    } else {
        ranges.visit_file(&file);
    }
    let mut bytes = source.as_bytes().to_vec();
    for range in ranges.0 {
        assert!(range.end <= bytes.len(), "Rust span exceeds source length");
        for byte in &mut bytes[range] {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
    }
    Ok(String::from_utf8(bytes).expect("masked complete Rust syntax retains UTF-8"))
}
