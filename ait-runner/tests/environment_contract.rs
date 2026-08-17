use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ait_runner::environment_contract::{ENVIRONMENT_VARIABLES, REMOVED_ENVIRONMENT_NAMES};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source_files(path: &Path) -> Vec<PathBuf> {
    fn visit(path: &Path, output: &mut Vec<PathBuf>) {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return;
        };
        if metadata.file_type().is_symlink() {
            return;
        }
        if metadata.is_file() {
            output.push(path.to_path_buf());
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            visit(&entry.path(), output);
        }
    }

    let mut output = Vec::new();
    visit(path, &mut output);
    output.sort();
    output
}

fn ait_tokens(source: &str) -> BTreeSet<&str> {
    let bytes = source.as_bytes();
    let mut tokens = BTreeSet::new();
    let mut index = 0usize;
    while index + 4 <= bytes.len() {
        if &bytes[index..index + 4] != b"AIT_" {
            index += 1;
            continue;
        }
        let start = index;
        index += 4;
        while index < bytes.len()
            && (bytes[index].is_ascii_uppercase()
                || bytes[index].is_ascii_digit()
                || bytes[index] == b'_')
        {
            index += 1;
        }
        tokens.insert(&source[start..index]);
    }
    tokens
}

#[test]
fn executable_and_release_sources_use_only_the_unified_registry() {
    let root = repository_root();
    let mut paths = source_files(&root.join("src"));
    paths.extend(source_files(&root.join("ci")));
    paths.push(root.join("Cargo.toml"));
    paths.sort();
    paths.dedup();

    let removed = REMOVED_ENVIRONMENT_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut violations = BTreeSet::new();
    for path in paths {
        if path.ends_with("src/environment_contract.rs") {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        for token in ait_tokens(&source) {
            if removed.contains(token) {
                violations.insert(format!(
                    "{} still contains removed {token}",
                    path.strip_prefix(&root).unwrap_or(&path).display()
                ));
                continue;
            }
            if !ENVIRONMENT_VARIABLES
                .iter()
                .copied()
                .any(|entry| entry.matches(token))
            {
                violations.insert(format!(
                    "{} contains unregistered {token}",
                    path.strip_prefix(&root).unwrap_or(&path).display()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "runner environment contract drift:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn release_manifest_tokens_do_not_expand_the_environment_contract() {
    let root = repository_root();
    let manifest =
        fs::read_to_string(root.join("ait-release.json")).expect("read release adapter manifest");
    let tokens = ait_tokens(&manifest);

    assert!(tokens.contains("AIT_RELEASE_TARGET"));
    assert!(
        ENVIRONMENT_VARIABLES
            .iter()
            .all(|entry| !entry.matches("AIT_RELEASE_TARGET"))
    );
    assert!(REMOVED_ENVIRONMENT_NAMES.contains(&"AIT_RELEASE_TARGET"));
}

#[test]
fn clap_has_no_environment_configuration_feature_or_argument_fallbacks() {
    let root = repository_root();
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml"))
            .expect("parse Cargo.toml");
    let features = manifest["dependencies"]["clap"]["features"]
        .as_array()
        .expect("clap features");
    assert!(
        features
            .iter()
            .all(|feature| feature.as_str() != Some("env"))
    );

    let main = fs::read_to_string(root.join("src/main.rs")).expect("read main.rs");
    assert!(!main.contains("#[arg(long, env"));
}
