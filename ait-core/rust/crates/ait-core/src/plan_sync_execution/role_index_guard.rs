//! Reject ambiguous role indexes before Plan sync can persist a revision.
use super::{SyncRequest, SyncTarget};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub(super) fn validate_before_sync(
    request: &SyncRequest,
    target: &SyncTarget,
) -> Result<(), String> {
    let role_dir = Path::new(&request.root_path).join("docs/roles");
    let targets_roles = target.target_path == "docs/roles"
        || target.target_path.starts_with("docs/roles/")
        || target.files.iter().any(|path| path.starts_with(&role_dir));
    if !targets_roles || !role_dir.exists() {
        return Ok(());
    }
    validate_role_directory(&role_dir)
}

pub fn validate_role_directory(role_dir: &Path) -> Result<(), String> {
    let mut names = fs::read_dir(role_dir)
        .map_err(|error| format!("Cannot inspect docs/roles before Plan sync: {error}"))?
        .map(|entry| {
            entry.map_err(|error| format!("Cannot inspect docs/roles before Plan sync: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort_by_key(|entry| entry.file_name());
    let mut by_index = BTreeMap::<u32, String>::new();
    let mut invalid = Vec::new();
    for entry in names {
        let path = entry.path();
        if !path.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or("Role Markdown filename is not UTF-8")?;
        let stem = &name[..name.len() - 3];
        let Some((number, role)) = stem.split_once('_') else {
            if stem.starts_with(|ch: char| ch.is_ascii_digit()) {
                invalid.push(name.to_string());
            }
            continue;
        };
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            continue; // Existing unnumbered role files belong to the old route.
        }
        let index = number
            .parse::<u32>()
            .map_err(|_| format!("Role index is out of range in docs/roles/{name}"))?;
        if index == u32::MAX {
            return Err(format!("Role index is out of range in docs/roles/{name}"));
        }
        if let Some(previous) = by_index.insert(index, name.to_string()) {
            return Err(format!(
                "Duplicate role index {index} in docs/roles/{previous} and docs/roles/{name}; Plan sync stopped before writing a revision"
            ));
        }
        if (number.len() > 1 && number.starts_with('0'))
            || role.is_empty()
            || !name.ends_with(".md")
        {
            invalid.push(name.to_string());
        }
    }
    if let Some(name) = invalid.first() {
        return Err(format!(
            "Invalid indexed role filename docs/roles/{name}; use <index>_<role>.md with a unique, non-padded decimal index"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_numeric_indexes_and_leading_zero_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let roles = temp.path();
        fs::write(roles.join("1_Developer.md"), "# Developer\n").unwrap();
        fs::write(roles.join("01_Designer.md"), "# Designer\n").unwrap();
        let error = validate_role_directory(roles).unwrap_err();
        assert!(error.contains("Duplicate role index 1"), "{error}");
        assert!(error.contains("01_Designer.md"), "{error}");
        assert!(error.contains("1_Developer.md"), "{error}");

        fs::remove_file(roles.join("01_Designer.md")).unwrap();
        fs::write(roles.join("1_Designer.md"), "# Designer\n").unwrap();
        assert!(validate_role_directory(roles)
            .unwrap_err()
            .contains("Duplicate role index 1"));

        fs::remove_file(roles.join("1_Designer.md")).unwrap();
        fs::write(roles.join("1_Designer.MD"), "# Designer\n").unwrap();
        assert!(validate_role_directory(roles)
            .unwrap_err()
            .contains("Duplicate role index 1"));
    }

    #[test]
    fn keeps_legacy_unnumbered_files_and_rejects_invalid_indexed_names() {
        let temp = tempfile::tempdir().unwrap();
        let roles = temp.path();
        fs::write(roles.join("coordinator.md"), "# Legacy\n").unwrap();
        fs::write(roles.join("0_Project Lead.md"), "# Lead\n").unwrap();
        validate_role_directory(roles).unwrap();

        fs::write(roles.join("02_Reviewer.md"), "# Reviewer\n").unwrap();
        assert!(validate_role_directory(roles)
            .unwrap_err()
            .contains("Invalid indexed role filename"));
    }
}
