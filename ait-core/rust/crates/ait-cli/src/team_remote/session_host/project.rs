//! Native sidebar project identity follows the authoritative repository directory.
use ait_core::json_support::{json, JsonValue};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};

pub(crate) fn project_folder_name(root: &Path) -> Result<&str, String> {
    if !root.is_absolute() {
        return Err("Team project directory must be absolute".into());
    }
    root.file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Team project directory must have a UTF-8 folder name".into())
}

pub(super) fn ensure_project(
    root: &Path,
    mut rpc: impl FnMut(&str, JsonValue) -> Result<JsonValue, String>,
) -> Result<String, String> {
    let name = project_folder_name(root)?;
    let mut cursor = JsonValue::Null;
    let mut seen = HashSet::new();
    let mut matched = None;
    let mut shared_root = false;
    let mut complete = false;
    for _ in 0..64 {
        let page = rpc("project/list", json!({"cursor":cursor,"limit":100}))?;
        let projects = page["data"]
            .as_array()
            .ok_or("Invalid native project inventory")?;
        for project in projects {
            let roots = project["roots"]
                .as_array()
                .ok_or("Invalid native project roots")?;
            if roots.iter().any(|entry| entry["path"] == json!(root)) {
                if roots.len() != 1 {
                    shared_root = true;
                    continue;
                }
                if matched.is_some() {
                    return Err(
                        "Ambiguous native project roots; no project created or renamed".into(),
                    );
                }
                matched = Some(project.clone());
            }
        }
        match page.get("nextCursor") {
            Some(JsonValue::Null) => {
                complete = true;
                break;
            }
            Some(value) if value.as_str().is_some_and(|s| !s.is_empty()) => {
                if !seen.insert(value.as_str().unwrap().to_owned()) {
                    return Err("Native project inventory cursor repeated".into());
                }
                cursor = value.clone();
            }
            _ => return Err("Invalid native project cursor".into()),
        }
    }
    if !complete {
        return Err("Native project inventory incomplete".into());
    }
    if matched.is_none() && shared_root {
        return Err(
            "Only a shared multi-root project matches; reconcile without renaming it".into(),
        );
    }
    let project = if let Some(project) = matched {
        let id = project["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Missing native project ID")?;
        if project["name"] != json!(name) {
            rpc("project/update", json!({"projectId":id,"name":name}))?;
            {
                let verified = rpc("project/read", json!({"projectId":id}))?["project"].clone();
                if verified["id"] != json!(id) {
                    return Err("Native project identity changed during readback".into());
                }
                verified
            }
        } else {
            project
        }
    } else {
        // A lost create reply must reuse the same native project on retry.
        let key = format!(
            "ait-team-project-v1:{:x}",
            Sha256::digest(root.as_os_str().as_encoded_bytes())
        );
        let created = rpc(
            "project/create",
            json!({"name":name,"roots":[{"path":root}],"idempotencyKey":key}),
        )?;
        let id = created["project"]["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Missing created project ID")?;
        {
            let verified = rpc("project/read", json!({"projectId":id}))?["project"].clone();
            if verified["id"] != json!(id) {
                return Err("Native project identity changed during readback".into());
            }
            verified
        }
    };
    if project["name"] != json!(name) || project["roots"] != json!([{"path":root}]) {
        return Err("Native project directory/name readback did not match".into());
    }
    project["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or("Missing native project ID".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_directory_wins_over_names_and_pagination() {
        let root = Path::new("/workspace/my-app");
        let mut methods = vec![];
        let result=ensure_project(root, |method, params| {
            methods.push(method.to_owned());
            Ok(match method {
                "project/list" if params["cursor"].is_null() => json!({"data":[{"id":"wrong","name":"my-app","roots":[{"path":"/other/my-app"}]}],"nextCursor":"next"}),
                "project/list" => json!({"data":[{"id":"right","name":"old-name","roots":[{"path":root}]}],"nextCursor":null}),
                "project/update" => {assert_eq!(params,json!({"projectId":"right","name":"my-app"}));json!({})},
                "project/read" => json!({"project":{"id":"right","name":"my-app","roots":[{"path":root}]}}),
                _=>panic!("unexpected {method}"),
            })
        }).unwrap();
        assert_eq!(result, "right");
        assert_eq!(
            methods,
            [
                "project/list",
                "project/list",
                "project/update",
                "project/read"
            ]
        );
    }
    #[test]
    fn create_retry_uses_stable_key_and_checks_readback() {
        let mut keys = vec![];
        for _ in 0..2 {
            assert!(ensure_project(Path::new("/workspace/my-app"), |method, p| {
                Ok(match method {
                    "project/list" => json!({"data":[],"nextCursor":null}),
                    "project/create" => {
                        keys.push(p["idempotencyKey"].clone());
                        json!({"project":{"id":"saved"}})
                    }
                    "project/read" => json!({"project":{"id":"saved","name":"wrong","roots":[]}}),
                    _ => panic!("unexpected mutation"),
                })
            })
            .is_err());
        }
        assert_eq!(keys[0], keys[1]);
    }
    #[test]
    fn ambiguous_roots_and_cursor_loops_never_mutate() {
        for page in [
            json!({"data":[{"id":"multi","roots":[{"path":"/repo"},{"path":"/other"}]}],"nextCursor":null}),
            json!({"data":[{"id":"a","roots":[{"path":"/repo"}]},{"id":"b","roots":[{"path":"/repo"}]}],"nextCursor":null}),
            json!({"data":[],"nextCursor":"loop"}),
        ] {
            assert!(ensure_project(Path::new("/repo"), |m, _| {
                assert_eq!(m, "project/list");
                Ok(page.clone())
            })
            .is_err());
        }
    }
    #[test]
    fn exact_project_reuse_is_read_only_and_unnamed_root_is_rejected() {
        assert_eq!(ensure_project(Path::new("/repo"), |m,_| {
            assert_eq!(m,"project/list");Ok(json!({"data":[{"id":"same","name":"repo","roots":[{"path":"/repo"}]}],"nextCursor":null}))
        }).unwrap(), "same");
        assert!(project_folder_name(Path::new("/")).is_err());
        assert!(project_folder_name(Path::new("relative")).is_err());
    }
    #[test]
    fn dedicated_root_takes_precedence_over_shared_project() {
        assert_eq!(
            ensure_project(Path::new("/repo"), |m, _| {
                assert_eq!(m, "project/list");
                Ok(json!({"data":[
                {"id":"shared","name":"other","roots":[{"path":"/other"},{"path":"/repo"}]},
                {"id":"dedicated","name":"repo","roots":[{"path":"/repo"}]}
            ],"nextCursor":null}))
            })
            .unwrap(),
            "dedicated"
        );
    }
}
