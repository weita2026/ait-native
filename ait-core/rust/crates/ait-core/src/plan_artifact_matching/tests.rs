use super::*;

#[test]
fn artifact_identity_and_matching_semantics_follow_python_contract() {
    assert_eq!(artifact_blob_id("# Demo\n"), "BLB-31ca6c61ca3fcc54029a");

    let indexed = index_plans_by_artifact_identity(&json!([
        {
            "plan_id": "PL-1",
            "head_revision": {
                "artifact_path": "docs/sprints/demo.md",
                "artifact_selector": "demo/root",
            },
        },
        {
            "plan_id": "PL-2",
            "head_revision": {
                "artifact_path": "docs/sprints/demo.md",
                "artifact_selector": null,
            },
        }
    ]))
    .unwrap();
    assert_eq!(indexed.as_array().unwrap().len(), 2);

    let plan = json!({
        "title": "Demo",
        "published_head_revision_id": "PR-REMOTE-1",
        "publication_state": "published",
        "head_revision": {
            "publication_state": "published",
            "artifact_path": "docs/sprints/demo.md",
            "artifact_selector": "demo/root",
            "artifact_heading": "Demo",
            "artifact_blob_id": artifact_blob_id("# Demo\n"),
            "items": [{"plan_item_ref": "demo/item"}],
        },
    });
    let artifact = json!({
        "artifact_path": "docs/sprints/demo.md",
        "artifact_selector": "demo/root",
        "artifact_heading": "Demo",
        "artifact_body": "# Demo\n",
        "items": [{"plan_item_ref": "demo/item"}],
    });
    assert!(local_plan_fully_published(&plan).unwrap());
    assert!(plan_heads_equivalent(&plan, &plan).unwrap());
    assert!(plan_matches_sync_artifact(&plan, &artifact, true).unwrap());
    let open_generic = open_generic_plans_matching_blob_id(
        &json!([
            {
                "plan_id": "PL-3",
                "status": "draft",
                "head_revision": {
                    "artifact_selector": null,
                    "artifact_blob_id": artifact_blob_id("# Demo\n"),
                }
            }
        ]),
        &artifact_blob_id("# Demo\n"),
    )
    .unwrap();
    assert_eq!(open_generic.as_array().unwrap().len(), 1);
}

#[test]
fn additional_matching_semantics_follow_python_contract() {
    let candidates = json!([
        {"plan_id": "PL-1", "status": "draft"},
        {"plan_id": "PL-2", "status": "archived"},
        {"plan_id": "PL-3", "status": "superseded"},
        {"plan_id": "PL-4", "status": "published"},
    ]);
    let open = artifact_candidates_open(&candidates).unwrap();
    assert_eq!(
        open,
        json!([
            {"plan_id": "PL-1", "status": "draft"},
            {"plan_id": "PL-4", "status": "published"},
        ])
    );

    assert_eq!(
        plan_artifact_identity_label("docs/sprints/demo.md", Some("demo/root")),
        "docs/sprints/demo.md [demo/root]"
    );
    assert_eq!(
        plan_artifact_identity_label("docs/sprints/demo.md", None),
        "docs/sprints/demo.md"
    );

    let indexed_by_path = index_plans_by_artifact_path(&json!([
        {
            "plan_id": "PL-1",
            "head_revision": {
                "artifact_path": "docs/sprints/demo.md",
                "artifact_selector": "demo/root",
            },
        },
        {
            "plan_id": "PL-2",
            "head_revision": {
                "artifact_path": "docs/sprints/demo.md",
                "artifact_selector": null,
            },
        },
        {
            "plan_id": "PL-3",
            "head_revision": {
                "artifact_path": "",
                "artifact_selector": null,
            },
        }
    ]))
    .unwrap();
    assert_eq!(
        indexed_by_path,
        json!({
            "docs/sprints/demo.md": [
                {
                    "plan_id": "PL-1",
                    "head_revision": {
                        "artifact_path": "docs/sprints/demo.md",
                        "artifact_selector": "demo/root",
                    },
                },
                {
                    "plan_id": "PL-2",
                    "head_revision": {
                        "artifact_path": "docs/sprints/demo.md",
                        "artifact_selector": null,
                    },
                }
            ]
        })
    );

    let selector_matches = open_plans_matching_selector(
        &json!([
            {
                "plan_id": "PL-1",
                "status": "draft",
                "head_revision": {"artifact_selector": "demo/root"},
            },
            {
                "plan_id": "PL-2",
                "status": "archived",
                "head_revision": {"artifact_selector": "demo/root"},
            },
            {
                "plan_id": "PL-3",
                "status": "draft",
                "head_revision": {"artifact_selector": "other/root"},
            }
        ]),
        "demo/root",
    )
    .unwrap();
    assert_eq!(
        selector_matches,
        json!([
            {
                "plan_id": "PL-1",
                "status": "draft",
                "head_revision": {"artifact_selector": "demo/root"},
            }
        ])
    );
}
