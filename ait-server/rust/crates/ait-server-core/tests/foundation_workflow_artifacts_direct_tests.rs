use ait_server_core::foundation::workflow_artifacts::{
    attestation_id_for_patchset, land_submission_id_for_change, release_artifact_media_type,
    release_formula_payload, review_summary_from_rows, sanitize_release_artifact_path,
    validate_release_artifact_pack, workflow_artifacts_json, RELEASE_ARTIFACT_PACK_FORMAT_V1,
    RELEASE_ARTIFACT_PACK_MANIFEST_ENTRY, WORKFLOW_ARTIFACTS_CONTRACT,
    WORKFLOW_ARTIFACTS_REFERENCE_MODULE,
};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::io::Write;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn run(operation: &str, payload: JsonValue) -> JsonValue {
    workflow_artifacts_json(operation, &payload).expect("workflow artifact shaping should succeed")
}

fn array_contains(value: &JsonValue, expected: &str) -> bool {
    value
        .as_array()
        .expect("value should be an array")
        .iter()
        .any(|item| item == expected)
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn release_pack_bytes(entry_name: &str, artifact_path: &str, data: &[u8]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let manifest = json!({
            "pack_format": RELEASE_ARTIFACT_PACK_FORMAT_V1,
            "entry_name": entry_name,
            "path": artifact_path,
        });
        writer
            .start_file(RELEASE_ARTIFACT_PACK_MANIFEST_ENTRY, options)
            .expect("manifest entry should start");
        writer
            .write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
            .expect("manifest should write");
        writer
            .start_file(entry_name, options)
            .expect("content entry should start");
        writer.write_all(data).expect("content should write");
        writer.finish().expect("zip should finish");
    }
    cursor.into_inner()
}

#[test]
fn workflow_artifact_contract_names_python_reference_and_supported_operations() {
    let value = run("contract", json!({}));
    assert_eq!(value["contract"], json!(WORKFLOW_ARTIFACTS_CONTRACT));
    assert_eq!(
        value["reference_module"],
        json!(WORKFLOW_ARTIFACTS_REFERENCE_MODULE)
    );
    assert_eq!(value["mutates_state"], json!(false));
    assert!(array_contains(&value["operations"], "release-row"));
    assert!(array_contains(
        &value["operations"],
        "ci-rollout-suite-checks"
    ));
    assert!(array_contains(
        &value["operations"],
        "land-submission-id-for-change"
    ));
    assert!(array_contains(&value["operations"], "review-summary"));
    assert!(array_contains(
        &value["operations"],
        "release-artifact-pack"
    ));
    assert!(array_contains(
        &value["operations"],
        "release-formula-payload"
    ));
}

#[test]
fn release_rows_shape_artifacts_formula_package_and_next_action() {
    assert_eq!(
        release_artifact_media_type("manifest", "ignored"),
        "application/json"
    );
    assert_eq!(
        release_artifact_media_type("", "dist/ait-1.0.tar.gz"),
        "application/gzip"
    );
    assert_eq!(
        release_artifact_media_type("formula", "Formula/ait.rb"),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        sanitize_release_artifact_path(Some("/tmp/dist/ait-1.0.tar.gz")),
        "dist/ait-1.0.tar.gz"
    );
    assert_eq!(sanitize_release_artifact_path(Some("  ")), "artifact");

    let value = run(
        "release-row",
        json!({
            "row": {
                "release_id": "REL-1",
                "line_name": "release/1",
                "package_name": "ait",
                "package_version": "1.0.0",
                "package_requires_python": ">=3.10",
                "checks_json": "not-json",
                "artifacts_json": serde_json::to_string(&json!([
                    {"kind": "sdist", "path": "/tmp/dist/ait-1.0.0.tar.gz", "sha256": "abc"},
                    {"kind": "formula", "path": "/tmp/homebrew/ait.rb", "download_name": ""}
                ])).unwrap(),
                "formula_json": serde_json::to_string(&json!({"artifact_kind": "sdist"})).unwrap(),
                "metadata_json": serde_json::to_string(&json!({
                    "package": {
                        "requires_python": ">=3.11",
                        "homepage": "https://example.invalid/ait",
                        "version": null
                    }
                })).unwrap()
            }
        }),
    );

    let release = &value["release"];
    assert_eq!(release["line"], json!("release/1"));
    assert_eq!(release["checks"], json!([]));
    assert_eq!(release["package"]["name"], json!("ait"));
    assert_eq!(release["package"]["version"], json!("1.0.0"));
    assert_eq!(release["package"]["requires_python"], json!(">=3.11"));
    assert_eq!(
        release["package"]["homepage"],
        json!("https://example.invalid/ait")
    );
    assert_eq!(
        release["artifacts"][0]["download_path"],
        json!("/v1/native/releases/REL-1/artifacts/sdist")
    );
    assert_eq!(
        release["artifacts"][0]["download_name"],
        json!("ait-1.0.0.tar.gz")
    );
    assert_eq!(release["artifacts"][1]["download_name"], json!("ait.rb"));
    assert_eq!(
        release["formula"]["url"],
        json!("/v1/native/releases/REL-1/artifacts/sdist")
    );
    assert_eq!(release["formula"]["sha256"], json!("abc"));
    assert_eq!(
        release["formula"]["download_path"],
        json!("/v1/native/releases/REL-1/artifacts/formula")
    );
    assert_eq!(release["next_action"]["code"], json!("published_remote"));
}

#[test]
fn release_artifact_pack_validation_extracts_content_and_storage_metadata() {
    let content = b"hello release\n";
    let artifact_path = "dist/ait-1.0.0.tar.gz";
    let entry_name = "payload.tar.gz";
    let pack_bytes = release_pack_bytes(entry_name, artifact_path, content);
    let artifact = json!({
        "kind": "SDIST",
        "path": format!("/tmp/{artifact_path}"),
        "sha256": sha256_hex(content),
        "size_bytes": content.len(),
        "content_entry_name": entry_name,
        "content_pack": {
            "pack_format": RELEASE_ARTIFACT_PACK_FORMAT_V1,
            "entry_name": entry_name,
            "bytes": pack_bytes
        }
    });
    let artifact_obj = artifact.as_object().expect("artifact should be object");
    let validation = validate_release_artifact_pack("REL-1", artifact_obj)
        .expect("artifact pack should validate");

    assert_eq!(validation.content, content);
    assert_eq!(validation.artifact["kind"], json!("sdist"));
    assert_eq!(validation.artifact["path"], json!(artifact_path));
    assert_eq!(validation.artifact["size_bytes"], json!(content.len()));
    assert_eq!(validation.artifact["sha256"], json!(sha256_hex(content)));
    assert_eq!(
        validation.artifact["download_name"],
        json!("ait-1.0.0.tar.gz")
    );
    assert_eq!(validation.artifact["media_type"], json!("application/gzip"));

    let value = run(
        "release-artifact-pack",
        json!({
            "release_id": "REL-1",
            "artifact": artifact,
            "include_content_bytes": true
        }),
    );
    assert_eq!(
        value["reference_module"],
        json!("../ait/src/ait_native/server_api.py")
    );
    assert_eq!(value["artifact"]["sha256"], json!(sha256_hex(content)));
    assert_eq!(
        value["content_bytes"],
        JsonValue::Array(content.iter().map(|byte| json!(byte)).collect())
    );
}

#[test]
fn release_artifact_pack_validation_rejects_manifest_and_digest_mismatches() {
    let content = b"hello release\n";
    let pack_bytes = release_pack_bytes("payload.tar.gz", "dist/right.tar.gz", content);
    let path_mismatch = validate_release_artifact_pack(
        "REL-1",
        json!({
            "kind": "sdist",
            "path": "dist/wrong.tar.gz",
            "content_pack": {
                "pack_format": RELEASE_ARTIFACT_PACK_FORMAT_V1,
                "entry_name": "payload.tar.gz",
                "bytes": pack_bytes
            }
        })
        .as_object()
        .unwrap(),
    )
    .expect_err("path mismatch should fail");
    assert_eq!(
        path_mismatch,
        "Release artifact sdist content_pack path mismatch"
    );

    let pack_bytes = release_pack_bytes("payload.tar.gz", "dist/right.tar.gz", content);
    let digest_mismatch = validate_release_artifact_pack(
        "REL-1",
        json!({
            "kind": "sdist",
            "path": "dist/right.tar.gz",
            "sha256": sha256_hex(b"wrong"),
            "content_pack": {
                "pack_format": RELEASE_ARTIFACT_PACK_FORMAT_V1,
                "entry_name": "payload.tar.gz",
                "bytes": pack_bytes
            }
        })
        .as_object()
        .unwrap(),
    )
    .expect_err("sha mismatch should fail");
    assert!(digest_mismatch.starts_with("Release artifact sdist sha256 mismatch:"));
}

#[test]
fn release_formula_payload_matches_publish_payload_shape() {
    let artifacts = vec![
        json!({"kind": "sdist", "path": "dist/pkg.tar.gz", "sha256": "abc"})
            .as_object()
            .unwrap()
            .clone(),
        json!({"kind": "formula", "path": "Formula/ait.rb"})
            .as_object()
            .unwrap()
            .clone(),
    ];
    let formula = json!({
        "name": "ait",
        "class_name": "Ait",
        "artifact_kind": "sdist",
        "sha256": "fallback"
    });
    let shaped = release_formula_payload(formula.as_object(), &artifacts);

    assert_eq!(shaped["name"], json!("ait"));
    assert_eq!(shaped["class_name"], json!("Ait"));
    assert_eq!(shaped["artifact_kind"], json!("sdist"));
    assert_eq!(shaped["path"], json!("Formula/ait.rb"));
    assert_eq!(shaped["sha256"], json!("abc"));

    let value = run(
        "release-formula-payload",
        json!({
            "formula": formula,
            "artifacts": artifacts.into_iter().map(JsonValue::Object).collect::<Vec<_>>()
        }),
    );
    assert_eq!(value["formula"], JsonValue::Object(shaped));
}

#[test]
fn patchset_paths_and_suite_catalog_helpers_match_reference_shape() {
    let changed = run(
        "patchset-changed-paths",
        json!({
            "patchset": {
                "diff_stats": {
                    "paths": {
                        "added": ["src/lib.rs", " docs/plan.md ", ""],
                        "deleted": ["old.py"],
                        "modified": ["src/lib.rs", "Cargo.toml"]
                    }
                }
            }
        }),
    );
    assert_eq!(
        changed["changed_paths"],
        json!(["Cargo.toml", "docs/plan.md", "old.py", "src/lib.rs"])
    );

    let catalog = run(
        "coerce-suite-catalog-payload",
        json!({
            "catalog_path": "ci/patch_ci.json",
            "payload": {
                "suites": [
                    {"suite_id": "unit", "plane": "patchset", "default_blocking": true},
                    {"suite_id": "", "plane": "patchset"},
                    {"suite_id": "docs", "plane": "release"}
                ]
            }
        }),
    );
    assert_eq!(
        catalog["suites"]["unit"]["_artifact_path"],
        json!("ci/patch_ci.json")
    );
    assert!(catalog["suites"].get("").is_none());

    let explicit = run(
        "suite-manifest-catalog-path",
        json!({"ci_config": {"suite_manifest_path": "ci/custom.json"}, "manifest": {}}),
    );
    assert_eq!(explicit["catalog_path"], json!("ci/custom.json"));
    let fallback = run(
        "suite-manifest-catalog-path",
        json!({"ci_config": {}, "manifest": {"ci/patch_ci.json": {"blob_id": "B1"}}}),
    );
    assert_eq!(fallback["catalog_path"], json!("ci/patch_ci.json"));

    let rollout = run(
        "patchset-rollout-suite-ids",
        json!({
            "suites_by_id": {
                "docs": {"plane": "release", "default_blocking": true},
                "lint": {"plane": "patchset", "default_blocking": false},
                "unit": {"plane": "patchset", "default_blocking": true}
            }
        }),
    );
    assert!(array_contains(&rollout["patchset_suite_ids"], "lint"));
    assert!(array_contains(&rollout["patchset_suite_ids"], "unit"));
    assert!(array_contains(&rollout["required_suite_ids"], "unit"));
    assert!(array_contains(&rollout["informational_suite_ids"], "lint"));
}

#[test]
fn ci_rollout_summary_and_suite_checks_preserve_status_messages() {
    let payload = json!({
        "phase": 1,
        "required_patchset_suites": ["unit"],
        "informational_patchset_suites": ["lint", "docs"],
        "promotion_candidates": {"phase1": ["full", "full"], "phase2": ["nightly"]},
        "suite_results_by_id": {
            "unit": {"status": "pass"},
            "lint": {"status": "fail"}
        }
    });
    let summary = run("ci-rollout-summary-message", payload.clone());
    assert_eq!(
        summary["message"],
        json!("CI rollout phase 1 blocks `unit` and keeps `lint`, `docs` visible as non-blocking surfaces. Future promotions are modeled as phase1: `full`, phase2: `nightly`.")
    );

    let checks = run("ci-rollout-suite-checks", payload);
    assert_eq!(checks["checks"][0]["name"], json!("ci_patchset_suite_unit"));
    assert_eq!(checks["checks"][0]["status"], json!("pass"));
    assert_eq!(checks["checks"][1]["name"], json!("ci_patchset_suite_lint"));
    assert_eq!(checks["checks"][1]["status"], json!("optional_fail"));
    assert_eq!(checks["checks"][2]["name"], json!("ci_patchset_suite_docs"));
    assert_eq!(checks["checks"][2]["status"], json!("not_required"));
}

#[test]
fn policy_review_and_id_helpers_fail_closed_and_keep_compat_shapes() {
    assert_eq!(
        run("review-decision-lane", json!({"action": "task_approve"}))["lane"],
        json!("task")
    );
    assert_eq!(
        run("review-decision-lane", json!({"action": "approve"}))["lane"],
        json!("team")
    );
    assert!(run("review-decision-lane", json!({"action": "comment"}))["lane"].is_null());

    assert_eq!(
        run(
            "requires-code-review-summary",
            json!({"content_class": "code_change", "author_class": "ai_related"})
        )["required"],
        json!(true)
    );
    assert_eq!(
        run(
            "requires-code-review-summary",
            json!({"effective_requirements": {"require_code_review_summary": true}})
        )["required"],
        json!(true)
    );

    let pending = run(
        "effective-policy-status",
        json!({
            "patchset": {"patchset_id": "RSEP-1", "evaluation_state": "pending"},
            "latest_status": {"patchset_id": "RSEP-1", "decision": "pending", "checks": [{"name": "tests"}]}
        }),
    );
    assert_eq!(pending["status"]["checks"][0]["name"], json!("tests"));

    let pass = run(
        "effective-policy-status",
        json!({
            "patchset": {"patchset_id": "RSEP-1", "evaluation_state": "pass"},
            "latest_status": {"patchset_id": "RSEP-1", "decision": "pending", "checks": []}
        }),
    );
    assert_eq!(pass["status"]["decision"], json!("pass"));

    assert_eq!(
        attestation_id_for_patchset(" RSEP-1 ").unwrap(),
        "AT-RSEP-1"
    );
    assert_eq!(
        land_submission_id_for_change(" RSEC-1 ", 2).unwrap(),
        "LAND-RSEC-1-0003"
    );
    assert!(land_submission_id_for_change("RSEC-1", -1).is_err());
}

#[test]
fn review_summary_matches_reference_counts_for_current_patchset() {
    let review_values = vec![
        json!({
            "review_id": 3,
            "patchset_id": "RSEP-OLD",
            "reviewer": "Old Reviewer",
            "action": "approve",
            "comment": "old patchset",
            "blocking": false
        }),
        json!({
            "review_id": 2,
            "patchset_id": "RSEP-1",
            "reviewer": "Alice",
            "action": "task_request_changes",
            "comment": "fix this",
            "blocking": true
        }),
        json!({
            "review_id": 1,
            "patchset_id": "RSEP-1",
            "reviewer": "Alice",
            "action": "task_approve",
            "comment": "previous decision",
            "blocking": false
        }),
        json!({
            "review_id": 4,
            "patchset_id": "RSEP-1",
            "reviewer": "Bob",
            "action": "approve",
            "comment": "ship it",
            "blocking": false
        }),
        json!({
            "review_id": 5,
            "patchset_id": "RSEP-1",
            "reviewer": "Bob",
            "action": "code_review_summary",
            "comment": "1. Reviewed files\nsrc/lib.rs\n2. Findings\nnone\n3. Risks\nlow\n4. Tests\ncargo test\n5. Recommendation\nland",
            "blocking": false
        }),
        json!({
            "review_id": 6,
            "patchset_id": "RSEP-1",
            "reviewer": "anonymous",
            "action": "task_approve",
            "comment": null,
            "blocking": false
        }),
        json!({
            "review_id": 7,
            "patchset_id": "RSEP-1",
            "reviewer": "Carol",
            "action": "comment",
            "comment": "FYI",
            "blocking": false
        }),
        json!({
            "review_id": 8,
            "patchset_id": "RSEP-1",
            "reviewer": "Dave",
            "action": "task_comment",
            "comment": "task note",
            "blocking": false
        }),
        json!({
            "review_id": 9,
            "patchset_id": "RSEP-1",
            "reviewer": "Eve",
            "action": "code_review_summary",
            "comment": "not structured",
            "blocking": false
        }),
        json!({
            "review_id": 10,
            "patchset_id": "RSEP-1",
            "reviewer": "Mallory",
            "action": "defer",
            "comment": "later",
            "blocking": false
        }),
    ];
    let reviews = review_values
        .iter()
        .map(|value| value.as_object().expect("review should be object").clone())
        .collect::<Vec<_>>();
    let summary = review_summary_from_rows(&reviews, Some("RSEP-1"));

    assert_eq!(summary["approval_count"], json!(2));
    assert_eq!(summary["task_approval_count"], json!(1));
    assert_eq!(summary["team_approval_count"], json!(1));
    assert_eq!(summary["human_approval_count"], json!(1));
    assert_eq!(summary["independent_human_approval_count"], json!(0));
    assert_eq!(summary["human_task_approval_count"], json!(0));
    assert_eq!(summary["independent_task_approval_count"], json!(0));
    assert_eq!(summary["code_review_summary_reviewer_count"], json!(1));
    assert_eq!(summary["blocking_count"], json!(1));
    assert_eq!(summary["comment_count"], json!(4));
    assert_eq!(summary["code_review_summary_count"], json!(1));
    assert_eq!(summary["review_count"], json!(9));
    assert_eq!(summary["approvals"], json!(2));
    assert_eq!(summary["comments"], json!(4));

    let value = run(
        "review-summary",
        json!({"patchset_id": "RSEP-1", "reviews": review_values}),
    );
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert!(value.get("reference_module").is_none());
    assert_eq!(value["summary"], JsonValue::Object(summary));
}
