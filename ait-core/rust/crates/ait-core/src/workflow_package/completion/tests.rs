use super::*;

#[test]
fn direct_exact_response_requires_every_distinct_canonical_change_mapping() {
    let mut source = vec![sources().remove(0)];
    source[0].changes = (1..=2)
        .map(|index| PackageChangeMappingExpectation {
            local_change_ref: format!("LCT-0001/C-{index:02}"),
            remote_change_ref: None,
        })
        .collect();
    let result = json!({"outcome":"confirmed","expected_identity":identity(),"patchset_id":"RCT-1001/P-01","submission_id":"RCT-1001/L-01","entries":[{"local_task_id":"LCT-0001","remote_task_id":"RCT-1001","changes":[{"local_change_ref":"LCT-0001/C-01","remote_change_ref":"RCT-1001/C-07"},{"local_change_ref":"LCT-0001/C-02","remote_change_ref":"RCT-1001/C-04"}]}]});
    let confirmed = PackageCompletion::from_value(&result, &identity(), &source).unwrap();
    assert_eq!(confirmed.outcome, PackageCompletionOutcome::Confirmed);
    assert!(confirmed.is_accepted());
    assert!(confirmed.has_complete_mappings());
    for fault in ["missing", "extra", "collision", "overflow"] {
        let mut changed = result.clone();
        let changes = changed["entries"][0]["changes"].as_array_mut().unwrap();
        match fault {
            "missing" => {
                changes.pop();
            }
            "extra" => {
                changes.push(
                    json!({"local_change_ref":"LCT-0001/C-03","remote_change_ref":"RCT-1001/C-11"}),
                );
            }
            "collision" => {
                changes[1]["remote_change_ref"] = json!("RCT-1001/C-07");
            }
            "overflow" => {
                changes[1]["remote_change_ref"] = json!("RCT-1001/C-65");
            }
            _ => unreachable!(),
        }
        assert!(
            PackageCompletion::from_value(&changed, &identity(), &source).is_err(),
            "{fault}"
        );
    }
}

#[test]
fn wrong_hash_stops_at_the_immutable_key_before_source_mapping_reads() {
    let mut wrong = identity();
    wrong.package_hash = "cd".repeat(32);
    let result = lookup_package_completion(&wrong, "main", &sources(), |resource| {
        if let PackageReadResource::Snapshot(ref id) = resource {
            assert_eq!(
                id,
                &identity().revision_snapshot_id,
                "wrong hash must not inspect/map source anchors"
            );
        }
        read(resource)
    });
    assert_eq!(result.outcome, PackageCompletionOutcome::Unknown);
    assert!(result.entries.is_empty());
}

#[test]
fn equal_task_counts_do_not_prove_missing_extra_or_substituted_source_members() {
    for fault in [
        "missing_snapshot",
        "substitute_snapshot",
        "extra_snapshot",
        "extra_change",
    ] {
        let mut source = sources();
        match fault {
            "missing_snapshot" => {
                source[0].snapshots.clear();
            }
            "substitute_snapshot" => {
                source[0].snapshots[0].snapshot_id = "SNP-000000000201".into();
            }
            "extra_snapshot" => {
                source[0].snapshots.push(PackageSnapshotOwnerExpectation {
                    snapshot_id: "SNP-000000000201".into(),
                    local_change_ref: None,
                });
            }
            "extra_change" => {
                source[0].changes.push(PackageChangeMappingExpectation {
                    local_change_ref: "LCT-0001/C-03".into(),
                    remote_change_ref: Some("RCT-1001/C-11".into()),
                });
            }
            _ => unreachable!(),
        }
        let result = lookup_package_completion(&identity(), "main", &source, |resource| {
            // Even a substituted anchor of the SAME Task cannot prove the
            // immutable package membership from these GET projections.
            if matches!(&resource,PackageReadResource::Snapshot(id) if id=="SNP-000000000201") {
                Ok(
                    json!({"snapshot_id":"SNP-000000000201","workflow_owner":{"task_id":"RCT-1001","change_ref":null}}),
                )
            } else {
                read(resource)
            }
        });
        assert_accepted_incomplete(&result);
        assert!(result.entries.is_empty());
    }
}

fn identity() -> PackageWorkflowExpectedIdentity {
    PackageWorkflowExpectedIdentity {
        repository_index: 0,
        worker_job_index: 0,
        package_hash: "ab".repeat(32),
        base_snapshot_id: "SNP-000000000001".into(),
        revision_snapshot_id: "SNP-000000000002".into(),
    }
}
fn sources() -> Vec<PackageMappingExpectation> {
    (1..=5)
        .map(|index| PackageMappingExpectation {
            local_task_id: format!("LCT-{index:04}"),
            remote_task_id: None,
            changes: Vec::new(),
            snapshots: vec![PackageSnapshotOwnerExpectation {
                snapshot_id: if index == 5 {
                    identity().revision_snapshot_id
                } else {
                    format!("SNP-{:012X}", 100 + index)
                },
                local_change_ref: None,
            }],
        })
        .collect()
}
fn receipt() -> Value {
    json!({"repository_index":0,"task_id":"RCT-1005","change_id":null,"change_ref":null,"submission_id":"RCT-1005/L-01",
        "patchset_id":"RCT-1005/P-01","status":"succeeded","target_line":"main","target_line_index":0,"mode":"direct",
        "base_snapshot_id":"SNP-000000000001","revision_snapshot_id":"SNP-000000000002",
        "landed_snapshot_id":"SNP-000000000002","pre_land_target_snapshot_id":"SNP-000000000001",
        "created_at":"2026-10-20T00:00:00Z","updated_at":"2026-10-20T00:00:00Z"})
}

fn assert_accepted_incomplete(result: &PackageCompletion) {
    assert_eq!(result.outcome, PackageCompletionOutcome::Incomplete);
    assert!(result.is_accepted());
    assert!(!result.has_complete_mappings());
    assert_eq!(result.patchset_id.as_deref(), Some("RCT-1005/P-01"));
    assert_eq!(result.submission_id.as_deref(), Some("RCT-1005/L-01"));
    assert!(result.entries.is_empty());
}

fn assert_unknown(result: &PackageCompletion) {
    assert_eq!(result.outcome, PackageCompletionOutcome::Unknown);
    assert!(!result.is_accepted());
    assert!(!result.has_complete_mappings());
    assert!(result.patchset_id.is_none());
    assert!(result.submission_id.is_none());
    assert!(result.entries.is_empty());
}

fn edit_manifest(value: &mut Value, change: impl FnOnce(&mut Value)) {
    let mut manifest: Value = serde_json::from_str(
        value["summary"]
            .as_str()
            .unwrap()
            .strip_prefix("ait-history-promotion/v1 ")
            .unwrap(),
    )
    .unwrap();
    change(&mut manifest);
    value["summary"] = json!(format!("ait-history-promotion/v1 {manifest}"));
}
fn read(resource: PackageReadResource) -> Result<Value, String> {
    Ok(match resource {
        PackageReadResource::Job(index) => {
            assert_eq!(index, 0);
            json!({"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":9,
            "patchset_ci":{"ci_worker_job_index":99}},"package_execution":null})
        }
        PackageReadResource::Snapshot(id) => {
            let task = if id == identity().revision_snapshot_id {
                "RCT-1005".to_string()
            } else {
                format!(
                    "RCT-{}",
                    900 + u64::from_str_radix(id.strip_prefix("SNP-").unwrap(), 16).unwrap()
                )
            };
            json!({"snapshot_id":id,"workflow_owner":{"task_id":task,"change_ref":null}})
        }
        PackageReadResource::Task(id) => {
            if id == "RCT-1005" {
                json!({"task_id":id,"successful_land":receipt()})
            } else {
                json!({"task_id":id})
            }
        }
        PackageReadResource::Land(id) => {
            assert_eq!(id, "RCT-1005/L-01");
            receipt()
        }
        PackageReadResource::Patchset(id) => {
            let entries=(1..=5).map(|index|json!({"local_task_id":format!("LCT-{index:04}"),"task_id":format!("RCT-{}",1000+index),"local_change_ref":null,"change_ref":null})).collect::<Vec<_>>();
            let manifest = json!({"contract":"ait-history-promotion/v3","idempotency_key":format!("workflow-package:{}:0",identity().package_hash),"target_line":"main","base_snapshot_id":"SNP-000000000001",
                "revision_snapshot_id":"SNP-000000000002","entries":entries,"aggregate":{"task_id":"RCT-1005","patchset_id":id,"patchset_index":9}});
            json!({"patchset_id":id,"task_id":"RCT-1005","base_snapshot_id":"SNP-000000000001",
                "revision_snapshot_id":"SNP-000000000002","ci_worker_job_index":99,"summary":format!("ait-history-promotion/v1 {manifest}")})
        }
    })
}

#[test]
fn formal_joins_preserve_acceptance_without_complete_source_authority_after_cleanup() {
    let result = lookup_package_completion(&identity(), "main", &sources(), read);
    assert_accepted_incomplete(&result);
    assert_eq!(
        result.expected_identity.package_hash,
        identity().package_hash
    );
    assert_eq!(
        PackageCompletion::from_value(&json!(result), &identity(), &sources()).unwrap(),
        result
    );
}

#[test]
fn sealed_execution_availability_does_not_change_independent_formal_acceptance() {
    for execution in [
        Value::Null,
        json!({"expected_identity":identity(),"ci_state":"passed","availability":"available","recoverable":true}),
        json!({"expected_identity":identity(),"ci_state":"unknown","availability":"unavailable","recoverable":false}),
    ] {
        let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
            let mut value = read(resource.clone())?;
            if matches!(resource, PackageReadResource::Job(_)) {
                value["package_execution"] = execution.clone();
            }
            Ok::<_, String>(value)
        });
        assert_accepted_incomplete(&result);
    }
}

#[test]
fn formal_recovery_requires_the_original_job_index_in_the_immutable_aggregate() {
    for index in [Value::Null, json!(8), json!("9")] {
        let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
            let mut value = read(resource.clone())?;
            if matches!(resource, PackageReadResource::Patchset(_)) {
                let mut manifest: Value = serde_json::from_str(
                    value["summary"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("ait-history-promotion/v1 ")
                        .unwrap(),
                )
                .unwrap();
                manifest["aggregate"]["patchset_index"] = index.clone();
                value["summary"] = json!(format!("ait-history-promotion/v1 {manifest}"));
                value["patchset_index"] = json!(9);
            }
            Ok::<_, String>(value)
        });
        assert_eq!(result.outcome, PackageCompletionOutcome::Unknown, "{index}");
        assert!(result.entries.is_empty());
    }
    for index in [Value::Null, json!(8), json!(9)] {
        let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
            let mut value = read(resource.clone())?;
            if matches!(resource, PackageReadResource::Patchset(_)) {
                value["patchset_index"] = index.clone();
            }
            Ok::<_, String>(value)
        });
        if index == json!(9) {
            assert_accepted_incomplete(&result);
        } else {
            assert_unknown(&result);
        }
    }
}

#[test]
fn unavailable_transport_and_different_formal_identity_are_unknown_without_mappings() {
    let unreachable = lookup_package_completion(&identity(), "main", &sources(), |_| {
        Err::<Value, _>("transport unavailable")
    });
    assert_eq!(unreachable.outcome, PackageCompletionOutcome::Unknown);
    for fault in ["job", "land"] {
        let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
            let mut value = read(resource.clone())?;
            match resource {
                PackageReadResource::Job(_) if fault == "job" => {
                    value["job"]["patchset_index"] = json!(8)
                }
                PackageReadResource::Land(_) if fault == "land" => {
                    value["revision_snapshot_id"] = json!("SNP-000000000003")
                }
                _ => {}
            }
            Ok::<_, String>(value)
        });
        assert_eq!(result.outcome, PackageCompletionOutcome::Unknown, "{fault}");
        assert!(result.entries.is_empty());
        assert!(result.patchset_id.is_none());
    }
    assert!(unreachable.entries.is_empty());
}

#[test]
fn incomplete_requires_authoritative_available_precommit_execution_and_never_returns_mappings() {
    for available in [true, false] {
        let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
            assert!(matches!(resource, PackageReadResource::Job(0)));
            Ok::<_, String>(
                json!({"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":null},
                "package_execution":if available {json!({"expected_identity":identity(),"ci_state":"passed","availability":"available","recoverable":true})}else{Value::Null}}),
            )
        });
        assert_eq!(
            result.outcome,
            if available {
                PackageCompletionOutcome::Incomplete
            } else {
                PackageCompletionOutcome::Unknown
            }
        );
        assert!(result.entries.is_empty());
        assert!(!result.is_accepted());
        assert!(!result.has_complete_mappings());
        assert!(result.submission_id.is_none());
    }
}

#[test]
fn mixed_change_anchors_cannot_prove_the_complete_original_label_set() {
    let mut source = sources();
    source[0].changes = vec![
        PackageChangeMappingExpectation {
            local_change_ref: "LCT-0001/C-01".into(),
            remote_change_ref: None,
        },
        PackageChangeMappingExpectation {
            local_change_ref: "LCT-0001/C-02".into(),
            remote_change_ref: None,
        },
        PackageChangeMappingExpectation {
            local_change_ref: "LCT-0001/C-03".into(),
            remote_change_ref: Some("RCT-1001/C-11".into()),
        },
    ];
    source[0].snapshots[0].local_change_ref = Some("LCT-0001/C-01".into());
    source[0].snapshots.push(PackageSnapshotOwnerExpectation {
        snapshot_id: "SNP-000000000201".into(),
        local_change_ref: Some("LCT-0001/C-02".into()),
    });
    let fetch = |resource: PackageReadResource| match &resource {
        PackageReadResource::Snapshot(id) if id == "SNP-000000000065" => Ok(
            json!({"snapshot_id":id,"workflow_owner":{"task_id":"RCT-1001","change_ref":"RCT-1001/C-07"}}),
        ),
        PackageReadResource::Snapshot(id) if id == "SNP-000000000201" => Ok(
            json!({"snapshot_id":id,"workflow_owner":{"task_id":"RCT-1001","change_ref":"RCT-1001/C-04"}}),
        ),
        _ => read(resource),
    };
    let result = lookup_package_completion(&identity(), "main", &source, fetch);
    assert_accepted_incomplete(&result);
    assert!(result.entries.is_empty());
    for fault in ["missing", "task", "task_only", "collision", "conflict"] {
        let result = lookup_package_completion(&identity(), "main", &source, |resource| {
            let affected =
                matches!(&resource, PackageReadResource::Snapshot(id) if id == "SNP-000000000201");
            if affected && fault == "missing" {
                return Err("anchor unavailable".to_string());
            }
            let mut value = fetch(resource)?;
            if affected {
                match fault {
                    "task" => value["workflow_owner"]["task_id"] = json!("RCT-1002"),
                    "task_only" => value["workflow_owner"]["change_ref"] = Value::Null,
                    "collision" => value["workflow_owner"]["change_ref"] = json!("RCT-1001/C-07"),
                    "conflict" => value["workflow_owner"]["change_ref"] = json!("RCT-1001/C-11"),
                    _ => {}
                }
            }
            Ok(value)
        });
        assert_accepted_incomplete(&result);
        assert!(result.entries.is_empty());
    }
    source[0].changes[2].remote_change_ref = None;
    assert_eq!(
        lookup_package_completion(&identity(), "main", &source, fetch).outcome,
        PackageCompletionOutcome::Incomplete
    );
    source[0].snapshots.clear();
    assert_eq!(
        lookup_package_completion(&identity(), "main", &source, fetch).outcome,
        PackageCompletionOutcome::Incomplete
    );
}

#[test]
fn accepted_incomplete_uses_existing_wire_fields_without_authorizing_writeback() {
    let result = lookup_package_completion(&identity(), "main", &[], read);
    assert_accepted_incomplete(&result);
    let value = json!(result);
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(value["outcome"], "incomplete");
    assert_eq!(
        PackageCompletion::from_value(&value, &identity(), &[]).unwrap(),
        result
    );
    for fault in [
        "patchset_missing",
        "submission_missing",
        "patchset_empty",
        "foreign_patchset",
        "patchset_ordinal",
        "submission_ordinal",
        "unknown",
        "entries",
        "identity",
    ] {
        let mut changed = value.clone();
        match fault {
            "patchset_missing" => changed["patchset_id"] = Value::Null,
            "submission_missing" => changed["submission_id"] = Value::Null,
            "patchset_empty" => changed["patchset_id"] = json!(""),
            "foreign_patchset" => changed["patchset_id"] = json!("RCT-1004/P-01"),
            "patchset_ordinal" => changed["patchset_id"] = json!("RCT-1005/P-1"),
            "submission_ordinal" => changed["submission_id"] = json!("RCT-1005/L-65"),
            "unknown" => changed["outcome"] = json!("unknown"),
            "entries" => {
                changed["entries"] =
                    json!([{"local_task_id":"LCT-0005","remote_task_id":"RCT-1005","changes":[]}])
            }
            "identity" => changed["expected_identity"]["package_hash"] = json!("cd".repeat(32)),
            _ => unreachable!(),
        }
        assert!(
            PackageCompletion::from_value(&changed, &identity(), &[]).is_err(),
            "{fault}"
        );
    }
    assert!(PackageCompletion::from_value(
        &json!({"expected_identity":identity(),"patchset_id":"RCT-1005/P-01","submission_id":"RCT-1005/L-01"}),
        &identity(), &[],
    ).is_err());
}

#[test]
fn exact_get_join_proves_acceptance_before_any_source_mapping_read() {
    for fault in [
        "no_source",
        "missing_row",
        "missing_entries",
        "wrong_row",
        "missing_labels",
    ] {
        let mut source = sources();
        if fault == "no_source" {
            source.clear();
        }
        let mut reads = 0;
        let result = lookup_package_completion(&identity(), "main", &source, |resource| {
            reads += 1;
            if let PackageReadResource::Snapshot(ref id) = resource {
                assert_eq!(id, &identity().revision_snapshot_id);
            }
            if let PackageReadResource::Task(ref id) = resource {
                assert_eq!(id, "RCT-1005");
            }
            let mut value = read(resource.clone())?;
            if matches!(resource, PackageReadResource::Patchset(_)) {
                edit_manifest(&mut value, |manifest| match fault {
                    "missing_row" => {
                        manifest["entries"].as_array_mut().unwrap().pop();
                    }
                    "missing_entries" => {
                        manifest.as_object_mut().unwrap().remove("entries");
                    }
                    "wrong_row" => manifest["entries"][0]["task_id"] = json!("RCT-9999"),
                    "missing_labels" => {
                        manifest["entries"][0]
                            .as_object_mut()
                            .unwrap()
                            .remove("local_task_id");
                    }
                    _ => {}
                });
            }
            Ok::<_, String>(value)
        });
        assert_eq!(reads, 5, "{fault}");
        assert_accepted_incomplete(&result);
    }
}

#[test]
fn unavailable_or_failed_required_formal_reads_never_prove_acceptance() {
    for failed in ["job", "snapshot", "task", "land", "patchset"] {
        for transport_error in [true, false] {
            let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
                let name = match &resource {
                    PackageReadResource::Job(_) => "job",
                    PackageReadResource::Snapshot(_) => "snapshot",
                    PackageReadResource::Task(_) => "task",
                    PackageReadResource::Land(_) => "land",
                    PackageReadResource::Patchset(_) => "patchset",
                };
                if name == failed {
                    if transport_error {
                        return Err("formal read unavailable".into());
                    }
                    return Ok(json!({"ok":false,"data":read(resource)?}));
                }
                read(resource)
            });
            assert_unknown(&result);
        }
    }
}

#[test]
fn conflicting_formal_evidence_never_preserves_ids_or_confirms_mapping() {
    for fault in [
        "repository",
        "job",
        "tombstone",
        "job_patchset",
        "snapshot",
        "owner",
        "receipt_missing",
        "receipt_null",
        "receipt_multiple",
        "receipt_owner",
        "receipt_patchset",
        "receipt_submission",
        "receipt_status",
        "receipt_base",
        "receipt_revision",
        "receipt_target",
        "receipt_mode",
        "receipt_repository",
        "land_owner",
        "land_patchset",
        "land_submission",
        "land_status",
        "land_base",
        "land_revision",
        "land_target",
        "land_mode",
        "land_repository",
        "patchset_owner",
        "patchset_id",
        "patchset_base",
        "patchset_revision",
        "patchset_repository",
        "summary",
        "key_hash",
        "key_job",
        "manifest_base",
        "manifest_revision",
        "manifest_target",
        "aggregate_task",
        "aggregate_patchset",
        "aggregate_index",
    ] {
        let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
            let mut value = read(resource.clone())?;
            match resource {
                PackageReadResource::Job(_) => match fault {
                    "repository" => value["job"]["repository_index"] = json!(1),
                    "job" => value["job"]["worker_job_index"] = json!(1),
                    "tombstone" => value["job"]["tombstoned"] = json!(true),
                    "job_patchset" => value["job"]["patchset_index"] = json!(8),
                    _ => {}
                },
                PackageReadResource::Snapshot(_) => match fault {
                    "snapshot" => value["snapshot_id"] = json!("SNP-000000000003"),
                    "owner" => value["workflow_owner"]["task_id"] = json!("RCT-1004"),
                    _ => {}
                },
                PackageReadResource::Task(_) => {
                    match fault {
                        "receipt_missing" => {
                            value.as_object_mut().unwrap().remove("successful_land");
                        }
                        "receipt_null" => value["successful_land"] = Value::Null,
                        "receipt_multiple" => {
                            value["successful_land"] = json!([receipt(), receipt()])
                        }
                        _ => {}
                    }
                    mutate_receipt(
                        &mut value["successful_land"],
                        fault.strip_prefix("receipt_"),
                    );
                }
                PackageReadResource::Land(_) => {
                    mutate_receipt(&mut value, fault.strip_prefix("land_"))
                }
                PackageReadResource::Patchset(_) => {
                    match fault {
                        "patchset_owner" => value["task_id"] = json!("RCT-1004"),
                        "patchset_id" => value["patchset_id"] = json!("RCT-1004/P-01"),
                        "patchset_base" => value["base_snapshot_id"] = json!("SNP-000000000003"),
                        "patchset_revision" => {
                            value["revision_snapshot_id"] = json!("SNP-000000000003")
                        }
                        "patchset_repository" => value["repository_index"] = json!(1),
                        "summary" => value["summary"] = json!("no immutable aggregate"),
                        _ => {}
                    }
                    if fault != "summary" {
                        edit_manifest(&mut value, |manifest| match fault {
                            "key_hash" => {
                                manifest["idempotency_key"] =
                                    json!(format!("workflow-package:{}:0", "cd".repeat(32)))
                            }
                            "key_job" => {
                                manifest["idempotency_key"] =
                                    json!(format!("workflow-package:{}:1", identity().package_hash))
                            }
                            "manifest_base" => {
                                manifest["base_snapshot_id"] = json!("SNP-000000000003")
                            }
                            "manifest_revision" => {
                                manifest["revision_snapshot_id"] = json!("SNP-000000000003")
                            }
                            "manifest_target" => manifest["target_line"] = json!("other"),
                            "aggregate_task" => {
                                manifest["aggregate"]["task_id"] = json!("RCT-1004")
                            }
                            "aggregate_patchset" => {
                                manifest["aggregate"]["patchset_id"] = json!("RCT-1004/P-01")
                            }
                            "aggregate_index" => manifest["aggregate"]["patchset_index"] = json!(8),
                            _ => {}
                        });
                    }
                }
            }
            Ok::<_, String>(value)
        });
        assert_unknown(&result);
    }
}

fn mutate_receipt(value: &mut Value, fault: Option<&str>) {
    match fault {
        Some("owner") => value["task_id"] = json!("RCT-1004"),
        Some("patchset") => value["patchset_id"] = json!("RCT-1004/P-01"),
        Some("submission") => value["submission_id"] = json!("RCT-1004/L-01"),
        Some("status") => value["status"] = json!("failed"),
        Some("base") => value["base_snapshot_id"] = json!("SNP-000000000003"),
        Some("revision") => value["revision_snapshot_id"] = json!("SNP-000000000003"),
        Some("target") => value["target_line"] = json!("other"),
        Some("mode") => value["mode"] = json!("merge"),
        Some("repository") => value["repository_index"] = json!(1),
        _ => {}
    }
}

#[test]
fn confirmed_writeback_still_requires_the_complete_original_task_mapping() {
    let source = sources();
    let entries = (1..=5).map(|index| json!({"local_task_id":format!("LCT-{index:04}"),"remote_task_id":format!("RCT-{}",1000+index),"changes":[]})).collect::<Vec<_>>();
    let value = json!({"outcome":"confirmed","expected_identity":identity(),"patchset_id":"RCT-1005/P-01","submission_id":"RCT-1005/L-01","entries":entries});
    assert!(PackageCompletion::from_value(&value, &identity(), &source)
        .unwrap()
        .has_complete_mappings());
    for fault in [
        "missing",
        "extra",
        "substituted",
        "order",
        "duplicate",
        "existing_mapping",
        "empty",
    ] {
        let mut changed = value.clone();
        let mut expected = source.clone();
        match fault {
            "missing" => {
                changed["entries"].as_array_mut().unwrap().pop();
            }
            "extra" => {
                changed["entries"].as_array_mut().unwrap().push(
                    json!({"local_task_id":"LCT-0006","remote_task_id":"RCT-1006","changes":[]}),
                );
            }
            "substituted" => changed["entries"][0]["local_task_id"] = json!("LCT-0006"),
            "order" => changed["entries"].as_array_mut().unwrap().swap(0, 1),
            "duplicate" => changed["entries"][1]["remote_task_id"] = json!("RCT-1001"),
            "existing_mapping" => expected[0].remote_task_id = Some("RCT-2001".into()),
            "empty" => {
                changed["entries"] = json!([]);
                expected.clear();
            }
            _ => unreachable!(),
        }
        assert!(
            PackageCompletion::from_value(&changed, &identity(), &expected).is_err(),
            "{fault}"
        );
    }
}

#[test]
fn foreign_or_noncanonical_snapshot_and_formal_change_owners_remain_unknown() {
    for fault in [
        "snapshot_foreign_change",
        "snapshot_change_ordinal",
        "land_change",
        "patchset_change",
        "land_final",
        "land_target_index",
    ] {
        let result = lookup_package_completion(&identity(), "main", &sources(), |resource| {
            let mut value = read(resource.clone())?;
            match resource {
                PackageReadResource::Snapshot(_) => match fault {
                    "snapshot_foreign_change" => {
                        value["workflow_owner"]["change_ref"] = json!("RCT-1004/C-01")
                    }
                    "snapshot_change_ordinal" => {
                        value["workflow_owner"]["change_ref"] = json!("RCT-1005/C-65")
                    }
                    _ => {}
                },
                PackageReadResource::Land(_) => match fault {
                    "land_change" => value["change_ref"] = json!("RCT-1004/C-01"),
                    "land_final" => value["landed_snapshot_id"] = json!("SNP-000000000003"),
                    "land_target_index" => value["target_line_index"] = json!(1),
                    _ => {}
                },
                PackageReadResource::Patchset(_) if fault == "patchset_change" => {
                    value["change_ref"] = json!("RCT-1004/C-01")
                }
                _ => {}
            }
            Ok::<_, String>(value)
        });
        assert_unknown(&result);
    }
}
