use ait_core::environment_contract::{ENVIRONMENT_VARIABLES, REMOVED_ENVIRONMENT_NAMES};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[path = "support/production_source.rs"]
mod production_source;
use production_source::{production_source, test_only_files};

static AIT_LITERAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\"(AIT_[A-Z0-9_]+)\""#).expect("literal regex"));
static DIRECT_ACCESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:env::(?:var|var_os|set_var|remove_var)|\.env(?:_remove)?)\s*\(\s*\"AIT_[A-Z0-9_]+\""#,
    )
    .expect("direct environment access regex")
});
static DIRECT_DECLARATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"const\s+[A-Z0-9_]*(?:ENV|ENV_VAR|ENV_VARS)[A-Z0-9_]*\s*:[^=]+\s*=\s*(?:&\[\s*)?\"AIT_[A-Z0-9_]+\""#)
        .expect("direct environment declaration regex")
});
static ENV_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:AIT_[A-Z0-9_]+|CODEX_[A-Z0-9_]+|X_SLACK_[A-Z0-9_]+)\b")
        .expect("environment token regex")
});

fn scanned_production_source(path: &Path, source: &str) -> String {
    if environment_authority_file(path) {
        return String::new();
    }
    production_source(source).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn environment_authority_file(path: &Path) -> bool {
    path == Path::new("rust/crates/ait-core/src/environment_contract.rs")
        || path == Path::new("rust/crates/ait-core/tests/environment_contract.rs")
}

fn scanned_source_set(
    sources: &BTreeMap<PathBuf, String>,
    roots: &BTreeSet<PathBuf>,
) -> BTreeMap<PathBuf, String> {
    let test_only = test_only_files(sources, roots).expect("parse Rust module evidence");
    sources
        .iter()
        .map(|(path, source)| {
            (
                path.clone(),
                if test_only.contains(path) {
                    String::new()
                } else {
                    scanned_production_source(path, source)
                },
            )
        })
        .collect()
}

static PRODUCTION_SOURCES: LazyLock<BTreeMap<PathBuf, String>> = LazyLock::new(|| {
    let root = repository_root();
    let crates = root.join("rust/crates");
    let sources = source_files(&crates, &["rs"])
        .into_iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(&root).expect("relative Rust source path");
            relative
                .components()
                .any(|part| part.as_os_str() == "src")
                .then(|| {
                    (
                        relative.to_path_buf(),
                        fs::read_to_string(&path).expect("read Rust source"),
                    )
                })
        })
        .collect::<BTreeMap<_, _>>();
    let mut roots = BTreeSet::new();
    for manifest in source_files(&crates, &["toml"])
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
    {
        let parsed = fs::read_to_string(&manifest)
            .expect("read Cargo manifest")
            .parse::<toml::Value>()
            .expect("parse Cargo manifest");
        if parsed.get("package").is_none() {
            continue;
        }
        let package = manifest.parent().unwrap().strip_prefix(&root).unwrap();
        roots.insert(package.join("src/lib.rs"));
        roots.insert(package.join("src/main.rs"));
        if let Some(path) = parsed
            .get("lib")
            .and_then(|lib| lib.get("path"))
            .and_then(toml::Value::as_str)
        {
            roots.insert(package.join(path));
        }
        if let Some(bins) = parsed.get("bin").and_then(toml::Value::as_array) {
            for bin in bins {
                if let Some(path) = bin.get("path").and_then(toml::Value::as_str) {
                    roots.insert(package.join(path));
                }
            }
        }
        let bin_directory = package.join("src/bin");
        for path in sources
            .keys()
            .filter(|path| path.starts_with(&bin_directory))
        {
            let relative = path.strip_prefix(&bin_directory).unwrap();
            if relative.components().count() == 1
                || (relative.components().count() == 2
                    && relative.file_name().is_some_and(|name| name == "main.rs"))
            {
                roots.insert(path.clone());
            }
        }
    }
    scanned_source_set(&sources, &roots)
});

fn unregistered_names(source: &str, registry: &BTreeSet<&str>) -> BTreeSet<String> {
    AIT_LITERAL
        .captures_iter(source)
        .map(|capture| capture[1].to_string())
        .filter(|name| {
            !name.ends_with('_')
                && !matches!(
                    name.as_str(),
                    "AIT_EXTERNAL_SNAPSHOT" | "AIT_RAM" | "AIT_TOKEN"
                )
                && !registry.contains(name.as_str())
        })
        .collect()
}

fn direct_access_lines(source: &str) -> BTreeSet<usize> {
    DIRECT_ACCESS
        .find_iter(source)
        .chain(DIRECT_DECLARATION.find_iter(source))
        .map(|found| {
            source[..found.start()]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
        })
        .collect()
}

fn removed_names(source: &str) -> BTreeSet<&str> {
    ENV_TOKEN
        .find_iter(source)
        .map(|found| found.as_str())
        .filter(|name| *name != "AIT_RAM" && REMOVED_ENVIRONMENT_NAMES.contains(name))
        .collect()
}

#[test]
fn environment_inventory_sizes_are_frozen() {
    assert_eq!(ENVIRONMENT_VARIABLES.len(), 31);
    assert_eq!(REMOVED_ENVIRONMENT_NAMES.len(), 80);
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("ait-core crate is nested below the repository root")
        .to_path_buf()
}

fn source_files(root: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    fn visit(path: &Path, extensions: &[&str], output: &mut Vec<PathBuf>) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        if metadata.file_type().is_symlink() {
            return;
        }
        if metadata.is_file() {
            let extension = path.extension().and_then(|value| value.to_str());
            if extension.is_some_and(|value| extensions.contains(&value)) {
                output.push(path.to_path_buf());
            }
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            visit(&entry.path(), extensions, output);
        }
    }

    let mut output = Vec::new();
    visit(root, extensions, &mut output);
    output.sort();
    output
}

#[test]
fn production_rust_ait_names_are_registered() {
    let registry = ENVIRONMENT_VARIABLES
        .iter()
        .map(|entry| entry.name)
        .collect::<BTreeSet<_>>();
    let mut unknown = BTreeSet::new();

    for (relative, source) in PRODUCTION_SOURCES.iter() {
        for name in unregistered_names(source, &registry) {
            unknown.insert(format!("{}: {name}", relative.display()));
        }
    }

    assert!(
        unknown.is_empty(),
        "production Rust contains unregistered AIT environment names:\n{}",
        unknown.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn production_environment_access_uses_registry_constants() {
    let mut violations = BTreeSet::new();

    for (relative, source) in PRODUCTION_SOURCES.iter() {
        let lines = direct_access_lines(source);
        for (line_index, line) in source.lines().enumerate() {
            if lines.contains(&line_index) {
                violations.insert(format!(
                    "{}:{}: {}",
                    relative.display(),
                    line_index + 1,
                    line
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "production environment access duplicates registry string literals:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn removed_environment_names_do_not_reappear_in_executable_source() {
    let root = repository_root();
    let mut violations = BTreeSet::new();
    let scan_roots = ["rust", "ci", "release", "tests", ".github"];

    for scan_root in scan_roots {
        for path in source_files(
            &root.join(scan_root),
            &["rs", "sh", "ps1", "mjs", "js", "yml", "yaml"],
        ) {
            if environment_authority_file(path.strip_prefix(&root).expect("relative source path"))
                || path
                    .components()
                    .any(|part| matches!(part.as_os_str().to_str(), Some("target" | ".ait")))
            {
                continue;
            }
            let relative = path.strip_prefix(&root).expect("relative source path");
            let source = fs::read_to_string(&path).expect("read executable source");
            for (line_index, line) in source.lines().enumerate() {
                for name in removed_names(line) {
                    violations
                        .insert(format!("{}:{}: {name}", relative.display(), line_index + 1,));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "removed environment names reappeared in executable source:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn production_controls_reject_unknown_names_and_multiline_literal_access() {
    let source = scanned_production_source(
        Path::new("src/runtime.rs"),
        r#"
const CONTROL_ENV: &str = "AIT_TASK_ID";
fn runtime() {
    std::env::var(
        "AIT_TEST_UNREGISTERED_RUNTIME"
    );
}
"#,
    );
    assert_eq!(
        unregistered_names(&source, &BTreeSet::from(["AIT_TASK_ID"])),
        BTreeSet::from(["AIT_TEST_UNREGISTERED_RUNTIME".to_string()]),
    );
    assert_eq!(direct_access_lines(&source), BTreeSet::from([1, 3]));
}

#[test]
fn production_controls_mask_inline_test_items_and_statements() {
    let source = r#"
#[cfg(test)]
mod children { fn child() { std::env::var("AIT_PRIVATE_CHILD"); } }
#[cfg(all(test, unix))]
fn pause() { std::env::var("AIT_PRIVATE_PAUSE"); }
#[test]
fn standalone() { std::env::var("AIT_PRIVATE_STANDALONE"); }
fn runtime() {
    #[cfg(test)]
    if let Some(_) = std::env::var_os("AIT_PRIVATE_STATEMENT") {}
    #[cfg(test)]
    let _ = std::env::var("AIT_PRIVATE_LOCAL");
}
"#;
    let production = scanned_production_source(Path::new("src/runtime.rs"), source);
    assert!(unregistered_names(&production, &BTreeSet::new()).is_empty());
    assert!(direct_access_lines(&production).is_empty());
    assert_eq!(production.len(), source.len());
    assert_eq!(production.lines().count(), source.lines().count());
}

#[test]
fn production_controls_preserve_possible_non_test_cfg_branches() {
    let source = scanned_production_source(
        Path::new("src/runtime.rs"),
        r#"
#[cfg(any(test, unix))]
fn platform() { std::env::var("AIT_PLATFORM_RUNTIME"); }
#[cfg_attr(feature = "optional", cfg(test))]
fn optional() { std::env::var("AIT_OPTIONAL_RUNTIME"); }
#[cfg_attr(not(test), cfg(test))]
fn excluded() { std::env::var("AIT_PRIVATE_CFG_ATTR"); }
"#,
    );
    assert_eq!(
        unregistered_names(&source, &BTreeSet::new()),
        BTreeSet::from([
            "AIT_PLATFORM_RUNTIME".to_string(),
            "AIT_OPTIONAL_RUNTIME".to_string(),
        ]),
    );
    assert_eq!(direct_access_lines(&source).len(), 2);
}

#[test]
fn production_controls_require_module_evidence_for_test_files() {
    let source = r#"fn child() { std::env::var("AIT_PRIVATE_CHILD"); }"#;
    for path in [
        "src/tests.rs",
        "src/component/tests/child.rs",
        "tests/child.rs",
        "src/environment_contract.rs",
    ] {
        let production = scanned_production_source(Path::new(path), source);
        assert!(
            !unregistered_names(&production, &BTreeSet::new()).is_empty(),
            "{path}"
        );
        assert!(!direct_access_lines(&production).is_empty(), "{path}");
    }
    for gated in [false, true] {
        let sources = BTreeMap::from([
            (
                PathBuf::from("src/lib.rs"),
                format!("{}mod tests;", if gated { "#[cfg(test)] " } else { "" }),
            ),
            (
                PathBuf::from("src/tests.rs"),
                format!("mod child; {source}"),
            ),
            (PathBuf::from("src/tests/child.rs"), source.to_string()),
        ]);
        let production =
            scanned_source_set(&sources, &BTreeSet::from([PathBuf::from("src/lib.rs")]));
        for path in ["src/tests.rs", "src/tests/child.rs"] {
            assert_eq!(
                unregistered_names(&production[Path::new(path)], &BTreeSet::new()).is_empty(),
                gated,
                "{path}"
            );
            assert_eq!(
                direct_access_lines(&production[Path::new(path)]).is_empty(),
                gated,
                "{path}"
            );
        }
    }
}

#[test]
fn production_module_references_override_test_aliases_and_paths() {
    for production_use in [
        "#[path = \"tests.rs\"] mod runtime;",
        "include!(\"tests.rs\");",
        "#[cfg_attr(feature = \"runtime\", path = \"tests.rs\")] mod runtime;",
    ] {
        let sources = BTreeMap::from([
            (
                PathBuf::from("src/lib.rs"),
                format!("#[cfg(test)] mod tests; {production_use}"),
            ),
            (
                PathBuf::from("src/tests.rs"),
                "fn runtime() { std::env::var(\"AIT_TEST_UNREGISTERED_RUNTIME\"); }".to_string(),
            ),
        ]);
        let production =
            scanned_source_set(&sources, &BTreeSet::from([PathBuf::from("src/lib.rs")]));
        let source = &production[Path::new("src/tests.rs")];
        assert!(
            !unregistered_names(source, &BTreeSet::new()).is_empty(),
            "{production_use}"
        );
        assert!(!direct_access_lines(source).is_empty(), "{production_use}");
    }
}

#[test]
fn literal_includes_are_precise_and_expression_includes_remain_in_production_scope() {
    for include in [
        "include!(\"tests.rs\");",
        "include!(concat!(\"tests\", \".rs\"));",
        "include!(concat!(concat!(\"te\", \"st\"), 's', \".rs\"));",
        "std::include!(std::concat!(\"tests\", \".rs\"),);",
    ] {
        let sources = BTreeMap::from([
            (
                PathBuf::from("src/lib.rs"),
                format!("#[cfg(test)] mod tests; #[cfg(test)] mod unrelated; {include}"),
            ),
            (
                PathBuf::from("src/tests.rs"),
                "fn runtime() { std::env::var(\"AIT_TEST_UNREGISTERED_RUNTIME\"); }".into(),
            ),
            (
                PathBuf::from("src/unrelated.rs"),
                "fn private() { std::env::var(\"AIT_PRIVATE_TEST_ONLY\"); }".into(),
            ),
        ]);
        let production =
            scanned_source_set(&sources, &BTreeSet::from([PathBuf::from("src/lib.rs")]));
        let included = &production[Path::new("src/tests.rs")];
        assert_eq!(
            unregistered_names(included, &BTreeSet::new()),
            BTreeSet::from(["AIT_TEST_UNREGISTERED_RUNTIME".into()]),
            "{include}"
        );
        assert_eq!(direct_access_lines(included).len(), 1, "{include}");
        let unrelated = &production[Path::new("src/unrelated.rs")];
        if include == "include!(\"tests.rs\");" {
            assert!(unregistered_names(unrelated, &BTreeSet::new()).is_empty());
            assert!(direct_access_lines(unrelated).is_empty());
        } else {
            // No macro binding is inferred. Potential targets stay scanned,
            // including a file that also has a test-only module alias.
            assert_eq!(
                unregistered_names(unrelated, &BTreeSet::new()),
                BTreeSet::from(["AIT_PRIVATE_TEST_ONLY".into()])
            );
            assert_eq!(direct_access_lines(unrelated).len(), 1);
        }
    }
}

#[test]
fn shadowed_and_user_qualified_concat_preserve_production_controls() {
    for (binding, expression) in [
        (
            r#"macro_rules! concat { ($($t:tt)*) => { "other.rs" }; }"#,
            r#"concat!("tests", ".rs")"#,
        ),
        (
            r#"mod custom { macro_rules! replacement { ($($t:tt)*) => { "other.rs" }; } pub(crate) use replacement as concat; }"#,
            r#"custom::concat!("tests", ".rs")"#,
        ),
        (
            r#"mod custom { macro_rules! replacement { ($($t:tt)*) => { "other.rs" }; } pub(crate) use replacement as concat; } use custom::concat;"#,
            r#"concat!("tests", ".rs")"#,
        ),
        (
            r#"mod custom { macro_rules! replacement { ($($t:tt)*) => { "other.rs" }; } pub(crate) use replacement as concat; } use custom::*;"#,
            r#"crate::concat!("tests", ".rs")"#,
        ),
        (
            r#"mod std { macro_rules! replacement { ($($t:tt)*) => { "other.rs" }; } pub(crate) use replacement as concat; }"#,
            r#"std::concat!("tests", ".rs")"#,
        ),
        (
            r#"mod custom { macro_rules! replacement { ($($t:tt)*) => { "other.rs" }; } pub(crate) use replacement as concat; } use custom as std;"#,
            r#"std::concat!("tests", ".rs")"#,
        ),
    ] {
        let sources = BTreeMap::from([
            (
                PathBuf::from("src/lib.rs"),
                format!("#[cfg(test)] mod other; {binding} include!({expression});"),
            ),
            (PathBuf::from("src/tests.rs"), "fn harmless() {}".into()),
            (
                PathBuf::from("src/other.rs"),
                "fn runtime() { ::std::env::var(\"AIT_TEST_UNREGISTERED_RUNTIME\"); }".into(),
            ),
        ]);
        let production =
            scanned_source_set(&sources, &BTreeSet::from([PathBuf::from("src/lib.rs")]));
        let included = &production[Path::new("src/other.rs")];
        assert_eq!(
            unregistered_names(included, &BTreeSet::new()),
            BTreeSet::from(["AIT_TEST_UNREGISTERED_RUNTIME".into()]),
            "{binding}"
        );
        assert_eq!(direct_access_lines(included).len(), 1, "{binding}");
    }
}

#[test]
fn test_gated_concat_expressions_preserve_proven_test_only_files() {
    for expression in [
        r#"concat!("tests", ".rs")"#,
        r#"::std::concat!("tests", ".rs")"#,
        r#"custom::concat!("tests", ".rs")"#,
    ] {
        let sources = BTreeMap::from([
            (
                PathBuf::from("src/lib.rs"),
                format!("#[cfg(test)] mod tests; #[cfg(test)] include!({expression});"),
            ),
            (
                PathBuf::from("src/tests.rs"),
                "fn private() { std::env::var(\"AIT_PRIVATE_TEST_ONLY\"); }".into(),
            ),
        ]);
        let production =
            scanned_source_set(&sources, &BTreeSet::from([PathBuf::from("src/lib.rs")]));
        for source in production.values() {
            assert!(unregistered_names(source, &BTreeSet::new()).is_empty());
            assert!(direct_access_lines(source).is_empty());
        }
    }
}

#[test]
fn unresolved_production_includes_preserve_possible_production_controls() {
    for include in [
        "include!(runtime_path!());",
        "include!(concat!(\"\", runtime_path!()));",
    ] {
        for nested in [false, true] {
            let runtime =
                format!("macro_rules! runtime_path {{ () => {{ \"tests.rs\" }}; }} {include}");
            let mut sources = BTreeMap::from([
                (
                    PathBuf::from("src/lib.rs"),
                    format!(
                        "#[cfg(test)] mod tests; {}",
                        if nested { "mod caller;" } else { &runtime }
                    ),
                ),
                (
                    PathBuf::from("src/tests.rs"),
                    "fn runtime() { std::env::var(\"AIT_TEST_UNREGISTERED_RUNTIME\"); }".into(),
                ),
            ]);
            if nested {
                sources.insert(PathBuf::from("src/caller.rs"), runtime);
            }
            let production =
                scanned_source_set(&sources, &BTreeSet::from([PathBuf::from("src/lib.rs")]));
            let included = &production[Path::new("src/tests.rs")];
            assert_eq!(
                unregistered_names(included, &BTreeSet::new()),
                BTreeSet::from(["AIT_TEST_UNREGISTERED_RUNTIME".into()]),
                "{include}"
            );
            assert_eq!(direct_access_lines(included).len(), 1, "{include}");
        }
    }
}

#[test]
fn test_only_unresolved_includes_preserve_proven_test_exclusions() {
    let sources = BTreeMap::from([
        (
            PathBuf::from("src/lib.rs"),
            "#[cfg(test)] mod tests; #[cfg(test)] include!(runtime_path!());".into(),
        ),
        (
            PathBuf::from("src/tests.rs"),
            "include!(runtime_path!()); fn private() { std::env::var(\"AIT_PRIVATE_TEST_ONLY\"); }"
                .into(),
        ),
    ]);
    let production = scanned_source_set(&sources, &BTreeSet::from([PathBuf::from("src/lib.rs")]));
    for source in production.values() {
        assert!(unregistered_names(source, &BTreeSet::new()).is_empty());
        assert!(direct_access_lines(source).is_empty());
    }
}

#[test]
fn production_module_evidence_preserves_declared_roots_and_unknown_sources() {
    let sources = BTreeMap::from([
        (
            PathBuf::from("src/lib.rs"),
            "#[cfg(test)] #[path = \"tests.rs\"] mod child;".to_string(),
        ),
        (
            PathBuf::from("src/tests.rs"),
            "fn runtime() { std::env::var(\"AIT_TEST_UNREGISTERED_RUNTIME\"); }".to_string(),
        ),
        (
            PathBuf::from("src/tests/unknown.rs"),
            "fn runtime() { std::env::var(\"AIT_TEST_UNREGISTERED_RUNTIME\"); }".to_string(),
        ),
    ]);
    let production = scanned_source_set(
        &sources,
        &BTreeSet::from([PathBuf::from("src/lib.rs"), PathBuf::from("src/tests.rs")]),
    );
    for path in ["src/tests.rs", "src/tests/unknown.rs"] {
        assert!(!unregistered_names(&production[Path::new(path)], &BTreeSet::new()).is_empty());
        assert!(!direct_access_lines(&production[Path::new(path)]).is_empty());
    }
}

#[test]
fn production_controls_preserve_adjacent_code_and_unicode_offsets() {
    let source = "// 中文\n#[cfg(test)] fn child() { std::env::var(\"AIT_PRIVATE_CHILD\"); } fn runtime() { std::env::var(\"AIT_REAL_RUNTIME\"); }\n";
    let production = scanned_production_source(Path::new("src/runtime.rs"), source);
    assert_eq!(
        unregistered_names(&production, &BTreeSet::new()),
        BTreeSet::from(["AIT_REAL_RUNTIME".to_string()]),
    );
    assert_eq!(direct_access_lines(&production), BTreeSet::from([1]));
}

#[test]
fn removed_name_control_still_covers_test_only_source() {
    let source = r#"#[cfg(test)] fn child() { std::env::var("AIT_CORE_BACKEND"); }"#;
    assert_eq!(removed_names(source), BTreeSet::from(["AIT_CORE_BACKEND"]));
    assert_eq!(
        removed_names("export AIT_CORE_BACKEND=native"),
        BTreeSet::from(["AIT_CORE_BACKEND"]),
    );
    assert!(removed_names("AIT_RAM").is_empty());
}
