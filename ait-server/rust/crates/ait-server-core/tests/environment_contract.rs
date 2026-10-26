use ait_server_core::environment_contract::{ENVIRONMENT_VARIABLES, REMOVED_ENVIRONMENT_NAMES};
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("ait-server-core is nested below the repository root")
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

fn is_installed_release_source(relative: &Path) -> bool {
    let text = relative.to_string_lossy();
    let in_release_crate = text.starts_with("rust/crates/ait-server/src/");
    let in_core = text.starts_with("rust/crates/ait-server-core/src/");
    if !in_release_crate && !in_core {
        return false;
    }
    if text.contains("/src/bin/")
        || text.contains("/tests/")
        || text.ends_with("/tests.rs")
        || text.contains("/foundation/db/")
        || text.contains("/foundation/native_repositories/service/postgres/")
        || text.ends_with("/foundation/native_repositories/service/postgres.rs")
    {
        return false;
    }
    true
}

/// Only an actual leading inner attribute proves this entire source module is
/// absent from installed release builds. Naming or substring matches do not.
fn has_leading_test_only_attribute(source: &str) -> bool {
    let mut remaining = source.strip_prefix('\u{feff}').unwrap_or(source);
    loop {
        remaining = remaining.trim_start();
        if let Some(comment) = remaining.strip_prefix("//") {
            remaining = comment.split_once('\n').map_or("", |(_, rest)| rest);
            continue;
        }
        if remaining.starts_with("/*") {
            let bytes = remaining.as_bytes();
            let mut index = 2;
            let mut depth = 1;
            while index + 1 < bytes.len() && depth > 0 {
                match &bytes[index..index + 2] {
                    b"/*" => {
                        depth += 1;
                        index += 2;
                    }
                    b"*/" => {
                        depth -= 1;
                        index += 2;
                    }
                    _ => index += 1,
                }
            }
            if depth != 0 {
                return false;
            }
            remaining = &remaining[index..];
            continue;
        }
        return remaining.starts_with("#![cfg(test)]");
    }
}

#[test]
fn file_level_test_attribute_requires_a_real_inner_attribute() {
    for source in [
        "#![cfg(test)]\nmod test_only {}",
        "\u{feff}\n  #![cfg(test)]\n",
        "// license\n/* outer /* nested */ comment */\n#![cfg(test)]\n",
    ] {
        assert!(has_leading_test_only_attribute(source), "{source}");
    }
    for source in [
        "// #![cfg(test)]\nfn production() {}",
        "/* #![cfg(test)] */\nfn production() {}",
        "const TEXT:&str=\"#![cfg(test)]\";",
        "r#\"#![cfg(test)]\"#",
        "#[cfg(test)] mod tests {}\nfn production() {}",
        "#![cfg(any(test,feature=\"runtime\"))]\nfn production() {}",
        "#![cfg(not(test))]\nfn production() {}",
        "/* unclosed #![cfg(test)]",
        "fn production() { /* #![cfg(test)] */ }",
        "",
    ] {
        assert!(!has_leading_test_only_attribute(source), "{source}");
    }
}

#[test]
fn installed_release_environment_access_uses_registered_names() {
    let root = repository_root();
    let registry = ENVIRONMENT_VARIABLES
        .iter()
        .map(|entry| entry.name)
        .collect::<BTreeSet<_>>();
    let direct_literal = Regex::new(
        r#"(?:std::)?env::(?:var|var_os)\s*\(\s*\"(AIT_[A-Z0-9_]+|AITSERVER_[A-Z0-9_]+)\""#,
    )
    .expect("direct environment-access regex");
    let registry_reference = Regex::new(r"environment_contract::names::(AIT_[A-Z0-9_]+)")
        .expect("registry reference regex");
    let mut violations = BTreeSet::new();

    for path in source_files(&root.join("rust/crates"), &["rs"]) {
        let relative = path.strip_prefix(&root).expect("relative source path");
        if !is_installed_release_source(relative) || relative.ends_with("environment_contract.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read Rust source");
        if has_leading_test_only_attribute(&source) {
            continue;
        }
        for capture in direct_literal.captures_iter(&source) {
            violations.insert(format!(
                "{} reads literal {} instead of the registry constant",
                relative.display(),
                capture.get(1).expect("direct name").as_str()
            ));
        }
        for capture in registry_reference.captures_iter(&source) {
            let name = capture.get(1).expect("registry name").as_str();
            if !registry.contains(name) {
                violations.insert(format!(
                    "{} references unregistered {name}",
                    relative.display()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "installed release environment readers bypass the registry:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn removed_environment_names_do_not_reappear_in_executable_source() {
    let root = repository_root();
    let removed = REMOVED_ENVIRONMENT_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let token = Regex::new(r"\b(?:AIT_[A-Z0-9_]+|AITSERVER_[A-Z0-9_]+)\b")
        .expect("environment token regex");
    let mut violations = BTreeSet::new();

    for scan_root in ["rust", "ci"] {
        for path in source_files(
            &root.join(scan_root),
            &["rs", "sh", "ps1", "mjs", "js", "yml", "yaml"],
        ) {
            let relative = path.strip_prefix(&root).expect("relative source path");
            if relative.ends_with("environment_contract.rs")
                || relative
                    .components()
                    .any(|part| matches!(part.as_os_str().to_str(), Some("target" | ".ait")))
            {
                continue;
            }
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
    for path in source_files(&root, &["sh"]) {
        let relative = path.strip_prefix(&root).expect("relative source path");
        if relative.components().count() != 1 {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read root script");
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

    assert!(
        violations.is_empty(),
        "removed environment names reappeared in executable source:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n")
    );
}
