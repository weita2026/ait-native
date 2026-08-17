use ait_core::environment_contract::{ENVIRONMENT_VARIABLES, REMOVED_ENVIRONMENT_NAMES};
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

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
    let root = repository_root();
    let registry = ENVIRONMENT_VARIABLES
        .iter()
        .map(|entry| entry.name)
        .collect::<BTreeSet<_>>();
    let literal = Regex::new(r#"\"(AIT_[A-Z0-9_]+)\""#).expect("literal regex");
    let mut unknown = BTreeSet::new();

    for crate_dir in source_files(&root.join("rust/crates"), &["rs"])
        .into_iter()
        .filter(|path| path.components().any(|part| part.as_os_str() == "src"))
    {
        let relative = crate_dir.strip_prefix(&root).expect("relative source path");
        if relative.ends_with("environment_contract.rs")
            || relative.file_name().and_then(|value| value.to_str()) == Some("tests.rs")
            || relative
                .components()
                .any(|part| part.as_os_str() == "tests")
        {
            continue;
        }
        let source = fs::read_to_string(&crate_dir).expect("read Rust source");
        for capture in literal.captures_iter(&source) {
            let name = capture.get(1).expect("AIT literal capture").as_str();
            if name.ends_with('_')
                || matches!(name, "AIT_EXTERNAL_SNAPSHOT" | "AIT_RAM" | "AIT_TOKEN")
            {
                continue;
            }
            if !registry.contains(name) {
                unknown.insert(format!("{}: {name}", relative.display()));
            }
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
    let root = repository_root();
    let direct_access = Regex::new(
        r#"(?:env::(?:var|var_os|set_var|remove_var)|\.env(?:_remove)?)\s*\(\s*\"AIT_[A-Z0-9_]+\""#,
    )
    .expect("direct environment access regex");
    let direct_declaration = Regex::new(
        r#"const\s+[A-Z0-9_]*(?:ENV|ENV_VAR|ENV_VARS)[A-Z0-9_]*\s*:[^=]+\s*=\s*(?:&\[\s*)?\"AIT_[A-Z0-9_]+\""#,
    )
    .expect("direct environment declaration regex");
    let mut violations = BTreeSet::new();

    for path in source_files(&root.join("rust/crates"), &["rs"])
        .into_iter()
        .filter(|path| path.components().any(|part| part.as_os_str() == "src"))
    {
        let relative = path.strip_prefix(&root).expect("relative source path");
        if relative.ends_with("environment_contract.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read Rust source");
        for (line_index, line) in source.lines().enumerate() {
            if direct_access.is_match(line) || direct_declaration.is_match(line) {
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
    let token = Regex::new(r"\b(?:AIT_[A-Z0-9_]+|CODEX_[A-Z0-9_]+|X_SLACK_[A-Z0-9_]+)\b")
        .expect("environment token regex");
    let removed = REMOVED_ENVIRONMENT_NAMES
        .iter()
        .copied()
        .filter(|name| *name != "AIT_RAM")
        .collect::<BTreeSet<_>>();
    let mut violations = BTreeSet::new();
    let scan_roots = ["rust", "ci", "release", "tests", ".github"];

    for scan_root in scan_roots {
        for path in source_files(
            &root.join(scan_root),
            &["rs", "sh", "ps1", "mjs", "js", "yml", "yaml"],
        ) {
            if path.ends_with("environment_contract.rs")
                || path
                    .components()
                    .any(|part| matches!(part.as_os_str().to_str(), Some("target" | ".ait")))
            {
                continue;
            }
            let relative = path.strip_prefix(&root).expect("relative source path");
            let source = fs::read_to_string(&path).expect("read executable source");
            for (line_index, line) in source.lines().enumerate() {
                for found in token.find_iter(line) {
                    if removed.contains(found.as_str()) {
                        violations.insert(format!(
                            "{}:{}: {}",
                            relative.display(),
                            line_index + 1,
                            found.as_str()
                        ));
                    }
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
