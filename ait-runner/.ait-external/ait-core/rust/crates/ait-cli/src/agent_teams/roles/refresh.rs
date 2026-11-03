//! Shipped-default provenance is operational evidence; numbered Plans remain role authority.
use super::*;
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use ait_core::snapshot_merge::{merge_utf8_text_bytes, TextMergeOutcome};
use sha2::{Digest, Sha256};

// Exact pre-upgrade defaults from SNP-3D8139A81A54. Never infer a managed
// default from prose, a filename, a partial match or the current Plan's age.
// Reviewer also admits the exact preceding default from SNP-EB3214AAA5F3.
// Retain subsequent shipped defaults for verified history upgrades.
const PREVIOUS: [&[&str]; 4] = [
    &[
        "2c7947893dfd9766490fe555ce5fa0f8ddd2a3d39c792cc6b041624bc9da2a54",
        "5ada87ebf8a4c4fada34c0503d4b47b4d76f72e265841a386ea83e71e93ec571",
        "7b53b58c1b0b99450da3c511a9fea68ff82428606336828ee9c014816ab1318a",
        "2aa9d8507a6ece68cb858572a4fb4cb433d8de039b75561df97e444fc265341b",
    ],
    &[
        "cf5ed32442b87dab09e8f0c3afa399a58bc2f076b68d75f99924fec1cbb02216",
        "27cc9440be1c6d35cf12537aa0d85980518f9b6faf664f4d1ce2ff1a2e82b3ca",
        "8c92a90ec6bf73071a667e6831ad1e177bf4020e1b4891c992e7577eed0341df",
        "37ae77b2847242109ec97a0f9594567173a692fa7792b5005bcd23950dffa41d",
    ],
    &[
        "f45cceadd6356b8495f82fa40d82ae1b5768b14f285d7103407c03f065d9a119",
        "f7baf3496c78c6f4e4b3475688efd1b9469e18da69b581e853207c5b2ea41b9f",
        "aeb3c4cc8cf61dec7d70d743526c4c078eef173e16886bd674f8d190f933ef3a",
        "e1b70af94ff7167d5e74041a8d87210971697cb5a59eaf2d4d39339f6b2e2a0f",
        "e5750a56f1c02a1939afabda4d1ed9c8f72373f2d5b961eca73a7778acc638f1",
    ],
    &[
        "5c6679bc2fb6f26f356d2c052dda47f997f3d3e837a19016e95a6316d176f828",
        "ca4f81927dbc6e775ef5456f25b2e6e7c47f68d36f2f2cdfa402a6befca64882",
        "65df89b286c520f5b26917169b40d35bd11ff108b73e2974185293284de35426",
        "507ddd208e8df4617a7816bedba243dd2bf4330e793376bea80654623a6d80e4",
    ],
];
fn digest(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

fn historical_base(repo: &RepoRuntime, path: &str, index: usize) -> Result<Option<String>, String> {
    let stores = repo.binary_db_stores::<1>();
    let plans = stores.plans();
    let read = plans.begin_read_txn();
    let heads = plans
        .list_plans(&read, None, Some(path))
        .map_err(|e| e.to_string())?;
    if heads.len() > 1 {
        return Err("Role Plan identity is ambiguous".into());
    }
    let Some(head) = heads.first() else {
        return Ok(None);
    };
    let revisions = plans
        .list_plan_revisions(&read, head.plan_index)
        .map_err(|e| e.to_string())?;
    let blobs = stores.content().blobs().clone();
    let blob_read = blobs.begin_read_txn();
    for revision in revisions.iter().take(64) {
        if revision.record.is_tombstone() {
            continue;
        }
        let bytes = blobs
            .read_blob_bytes_for_id(
                &blob_read,
                &revision
                    .payload
                    .artifact_blob_id_text()
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?
            .ok_or("Historical role body missing")?;
        let body = String::from_utf8(bytes).map_err(|_| "Historical role is not UTF-8")?;
        if body == TEMPLATES[index].1 || PREVIOUS[index].contains(&digest(&body).as_str()) {
            return Ok(Some(body));
        }
    }
    Ok(None)
}

pub(super) fn synchronize(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let root = repo.authoritative_repo_root();
    let known = paths(&root)?;
    fs::create_dir_all(root.join("docs/roles")).map_err(|e| e.to_string())?;
    let receipt_path = root.join(".ait/team-session-bootstrap/role-defaults");
    fs::create_dir_all(receipt_path.parent().unwrap()).map_err(|e| e.to_string())?;
    let prior = super::super::sessions::read_json(&receipt_path)?;
    if !prior.is_null()
        && (prior["version"] != 1
            || prior["repository"] != json!(root)
            || prior["roles"].as_array().is_none())
    {
        return Err("Invalid role-default provenance; preserve it for recovery".into());
    }
    let mut records = Vec::new();
    let mut outcomes = Vec::new();
    let mut conflicts = Vec::new();
    for (index, (name, template)) in TEMPLATES.iter().enumerate() {
        let path = known
            .get(&(index as u32))
            .map(|(path, _)| path.clone())
            .unwrap_or_else(|| format!("docs/roles/{name}"));
        let absolute = root.join(&path);
        let missing = !absolute.exists();
        let local = if missing {
            template.to_string()
        } else {
            fs::read_to_string(&absolute).map_err(|e| e.to_string())?
        };
        let previous = prior["roles"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["role_index"] == index as u32));
        let base = if let Some(previous) = previous {
            let body = previous["default_body"]
                .as_str()
                .ok_or("Role-default body missing")?;
            if previous["artifact_path"] != path || previous["default_sha256"] != digest(body) {
                return Err("Role-default provenance identity/digest differs".into());
            }
            Some(body.to_owned())
        } else if local == *template || PREVIOUS[index].contains(&digest(&local).as_str()) {
            Some(local.clone())
        } else {
            historical_base(repo, &path, index)?
        };
        let (body, state) = if missing {
            (local.clone(), "initialized")
        } else if local == *template {
            (local.clone(), "current")
        } else if let Some(base) = &base {
            match merge_utf8_text_bytes(base.as_bytes(), local.as_bytes(), template.as_bytes()) {
                TextMergeOutcome::Merged(bytes) => (
                    String::from_utf8(bytes).map_err(|_| "Merged role is not UTF-8")?,
                    if base == template {
                        "customized"
                    } else {
                        "updated_default"
                    },
                ),
                _ => {
                    conflicts.push(json!({"role_index":index,"artifact_path":path,"reason":"customized_default_upgrade_conflict","installed_default_sha256":digest(template)}));
                    (local.clone(), "conflict")
                }
            }
        } else {
            conflicts.push(json!({"role_index":index,"artifact_path":path,"reason":"customized_role_has_no_verified_default_base","installed_default_sha256":digest(template)}));
            (local.clone(), "conflict")
        };
        if missing || body != local {
            fence(&absolute, &body)?;
        }
        if state != "conflict" {
            records.push(json!({"role_index":index,"artifact_path":path,"default_sha256":digest(template),"default_body":template}));
        } else if let Some(previous) = previous {
            records.push(previous.clone());
        }
        outcomes.push(json!({"role_index":index,"artifact_path":path,"default_state":state,"installed_default_sha256":digest(template)}));
    }
    // Always sync actual Markdown, including custom edits and conflicts. An old
    // saved Plan must never be delivered as though it represented the local file.
    for (index, (path, _)) in paths(&root)? {
        let request = json!({"root_path":root,"repo_name":repo.repo_name(),"target":path,"local":true,
            "created_by":repo.actor_identity(),"plan_storage":repo.plan_binary_db_storage_request::<1>()?});
        let result = execute_plan_sync_command_request_json(&request.to_string())?;
        if result["status"] != "ok" {
            return Err(result.to_string());
        }
        if let Some(outcome) = outcomes.iter_mut().find(|row| row["role_index"] == index) {
            outcome["saved_role_revision_index"] = json!(load(repo, index)?.revision);
        }
    }
    fence(
        &receipt_path,
        &json!({"version":1,"repository":root,"roles":records}).to_string(),
    )?;
    Ok(
        json!({"status":if conflicts.is_empty(){"ready"}else{"pending"},"roles":outcomes,"conflicts":conflicts}),
    )
}
