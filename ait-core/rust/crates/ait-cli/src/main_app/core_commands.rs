fn run_init(args: InitArgs) -> Result<(), String> {
    let payload = init_cmd(&InitRequest {
        root: env::current_dir().map_err(|err| err.to_string())?,
        name: args.name,
        default_line: args.default_line,
        policy_profile: args.policy_profile,
        default_author_mode: args.default_author_mode,
        default_model: args.default_model,
        repair_existing: args.repair_existing,
    })?;
    if args.json {
        print_json(&payload)?;
    } else {
        render_human_init(&payload);
    }
    Ok(())
}

fn run_install(args: InstallArgs) -> Result<(), String> {
    let payload = install_cmd(&InstallRequest {
        cwd: env::current_dir().map_err(|err| err.to_string())?,
        mode: args.mode,
        attach: args.attach,
        server_setup: args.server_setup,
        server_url: args.server_url,
        remote_name: args.remote_name,
        remote_repo_name: args.remote_repo_name,
        repo_name: args.repo_name,
        user_name: args.user_name,
        user_email: args.user_email,
        initialize: tri_state_flag(args.init, args.no_init),
        sprint: tri_state_flag(args.sprint, args.no_sprint),
        worker_name: args.worker_name,
        telegram_token: args.telegram_token,
        telegram_username: args.telegram_username,
        discord_application_id: args.discord_application_id,
        discord_bot_token: args.discord_bot_token,
        dry_run: args.dry_run,
        json_output: args.json,
        interactive: !args.json,
    })?;
    if args.json {
        print_json(&payload)?;
    } else {
        println!("{}", render_install_text(&payload));
    }
    Ok(())
}

fn tri_state_flag(yes: bool, no: bool) -> Option<bool> {
    if yes {
        Some(true)
    } else if no {
        Some(false)
    } else {
        None
    }
}

fn run_doctor(command: DoctorCommand) -> Result<ExitCode, String> {
    match command {
        DoctorCommand::MemoryRoot(args) => {
            let payload = doctor_memory_root(args.ensure)?;
            emit_doctor_result("ait-cli doctor memory-root", &payload, args.json)?;
        }
        DoctorCommand::RuntimeRoot(args) => {
            let repo = RepoRuntime::discover()?;
            let payload =
                doctor_runtime_root(&repo.authoritative_repo_root(), args.server_data.as_deref())?;
            emit_doctor_result("ait-cli doctor runtime-root", &payload, args.json)?;
        }
        DoctorCommand::Postgres(args) => {
            let payload = doctor_postgres(
                None,
                args.server_data.as_deref(),
                Some(args.backend.as_str()),
                args.dsn.as_deref(),
                args.content_schema.as_deref(),
                args.control_schema.as_deref(),
                args.connect,
            )?;
            emit_doctor_result("ait-cli doctor postgres", &payload, args.json)?;
        }
        DoctorCommand::PlanAuthority(args) => {
            let payload = doctor_plan_authority(args.backend.as_deref())?;
            emit_doctor_result("ait-cli doctor plan-authority", &payload, args.json)?;
        }
        DoctorCommand::PlanAuthorityWheel(args) => {
            let payload = doctor_plan_authority_wheel(
                args.wheel.as_deref(),
                args.repack_installed,
                args.smoke,
            )?;
            emit_doctor_result("ait-cli doctor plan-authority-wheel", &payload, args.json)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_gc(repo: RepoRuntime, command: GcCommand) -> Result<(), String> {
    let content_maintenance =
        repo.local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()?;
    match command {
        GcCommand::Stats(args) => {
            let _command_range = perfetto_range!("ait.cli.gc.stats.command");
            let payload = {
                let _range = perfetto_range!("ait.cli.gc.stats.compute");
                content_maintenance.storage_stats_with_options(LocalContentStatsOptions {
                    include_inventory: args.include_inventory,
                    compute_reachability: args.deep || args.include_inventory,
                })?
            };
            let _range = perfetto_range!("ait.cli.gc.stats.render");
            emit_gc_payload("ait-cli gc stats", &payload, args.json)
        }
        GcCommand::Validate(args) => {
            let payload = content_maintenance.validate()?;
            emit_gc_payload("ait-cli gc validate", &payload, args.json)
        }
        GcCommand::Prune(args) => {
            let payload = content_maintenance.prune_orphan_packs()?;
            emit_gc_payload("ait-cli gc prune", &payload, args.json)
        }
    }
}

fn emit_gc_payload(title: &str, payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "gc payload must decode to an object.".to_string())?;
    let validation = obj
        .get("validation_summary")
        .and_then(JsonValue::as_object)
        .or_else(|| {
            obj.get("final_storage")
                .and_then(|value| value.get("validation_summary"))
                .and_then(JsonValue::as_object)
        })
        .or_else(|| {
            obj.get("stats")
                .and_then(|value| value.get("validation_summary"))
                .and_then(JsonValue::as_object)
        });
    print_key_values(
        title,
        &[
            (
                "state",
                string_field(
                    validation
                        .and_then(|value| value.get("state"))
                        .or_else(|| obj.get("state")),
                ),
            ),
            (
                "recommended_action",
                string_field(
                    validation
                        .and_then(|value| value.get("recommended_action"))
                        .or_else(|| obj.get("recommended_action")),
                ),
            ),
            (
                "pack_count",
                string_field(
                    obj.get("pack_count")
                        .or_else(|| {
                            obj.get("final_storage")
                                .and_then(|value| value.get("pack_count"))
                        })
                        .or_else(|| obj.get("stats").and_then(|value| value.get("pack_count"))),
                ),
            ),
            (
                "packed_delta_blob_count",
                string_field(
                    obj.get("packed_delta_blob_count")
                        .or_else(|| {
                            obj.get("final_storage")
                                .and_then(|value| value.get("packed_delta_blob_count"))
                        })
                        .or_else(|| {
                            obj.get("stats")
                                .and_then(|value| value.get("packed_delta_blob_count"))
                        }),
                ),
            ),
            ("created", string_field(obj.get("created"))),
            ("pack_id", string_field(obj.get("pack_id"))),
            (
                "removed_orphan_pack_count",
                string_field(obj.get("removed_orphan_pack_count")),
            ),
            (
                "executed_step_count",
                string_field(obj.get("executed_step_count")),
            ),
        ],
    );
    Ok(())
}

fn run_current_source_cache(command: CurrentSourceCacheCommand) -> Result<(), String> {
    match command {
        CurrentSourceCacheCommand::RunCli(_) => {
            unreachable!("current-source-cache run-cli is handled before command dispatch")
        }
        CurrentSourceCacheCommand::Contract(args) => {
            let payload =
                current_source_native_cache_contract_cmd(&CurrentSourceNativeCacheRequest {
                    namespace_root: args.path.namespace_root,
                    core_repo_root: args.path.core_repo_root,
                    core_source_fingerprint: args.path.core_source_fingerprint,
                    server_source_fingerprint: args.path.server_source_fingerprint,
                    ext_suffix: args.path.ext_suffix,
                    rustflags: args.path.rustflags,
                    worker_id: args.path.worker_id,
                })?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache contract",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::MarkBuilding(args) => {
            let paths = current_source_cache_paths_from_args(&args.path)?;
            let payload = write_current_source_native_cache_manifest_json(
                &CurrentSourceNativeCacheManifestRequest {
                    paths,
                    state: "building".to_string(),
                    source_mtime_ns: args.source_mtime_ns,
                    last_used_at: None,
                    size_bytes: None,
                    extra: parse_json_object_arg(&args.extra_json, "--extra-json")?,
                },
            )?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache mark-building",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::Activate(args) => {
            let _activate_range =
                ait_core::perfetto_range!("ait.core.current_source_cache.activate");
            let paths = current_source_cache_paths_from_args(&args.path)?;
            let mut payload = write_current_source_native_cache_manifest_json(
                &CurrentSourceNativeCacheManifestRequest {
                    paths: paths.clone(),
                    state: "ready".to_string(),
                    source_mtime_ns: args.source_mtime_ns,
                    last_used_at: None,
                    size_bytes: None,
                    extra: parse_json_object_arg(&args.extra_json, "--extra-json")?,
                },
            )?;
            let manifest = payload.clone();
            let lease = if args.register_lease {
                Some(match args.owner_pid {
                    Some(owner_pid) => register_current_source_native_cache_lease_for_owner_json(
                        &paths,
                        &args.path.worker_id,
                        owner_pid,
                    )?,
                    None => register_current_source_native_cache_lease_json(
                        &paths,
                        &args.path.worker_id,
                    )?,
                })
            } else {
                None
            };
            let prune =
                prune_current_source_native_caches_json(&CurrentSourceNativeCachePruneRequest {
                    namespace_root: paths.namespace_root.clone(),
                    now: None,
                    idle_ttl_seconds: CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS,
                    build_stale_seconds: CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS,
                    max_bytes: CURRENT_SOURCE_CACHE_MAX_BYTES,
                    remove_unleased_ready: false,
                })?;
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("manifest".to_string(), manifest);
                if let Some(lease) = lease {
                    obj.insert("lease".to_string(), lease);
                }
                obj.insert("prune".to_string(), prune);
            }
            emit_current_source_cache_payload(
                "ait-cli current-source-cache activate",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::ReleaseLease(args) => {
            let payload = release_current_source_native_cache_lease_json(
                &args.lease_path,
                &args.namespace_root,
                args.remove_unleased_ready,
            )?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache release-lease",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::Prune(args) => {
            let payload =
                prune_current_source_native_caches_json(&CurrentSourceNativeCachePruneRequest {
                    namespace_root: args.namespace_root,
                    now: args.now,
                    idle_ttl_seconds: args.idle_ttl_seconds,
                    build_stale_seconds: args.build_stale_seconds,
                    max_bytes: args.max_bytes,
                    remove_unleased_ready: args.remove_unleased_ready,
                })?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache prune",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::SeedCanonical(args) => {
            let core_source_fingerprint =
                args.path.core_source_fingerprint.clone().ok_or_else(|| {
                    "current-source-cache seed-canonical requires --core-source-fingerprint."
                        .to_string()
                })?;
            let payload = seed_current_source_native_cache_from_canonical_json(
                &CurrentSourceNativeCacheCanonicalSeedRequest {
                    namespace_root: args.path.namespace_root,
                    core_repo_root: args.path.core_repo_root,
                    repo_root: args.repo_root,
                    canonical_repo_root: args.canonical_repo_root,
                    core_source_mtime_ns: args.core_source_mtime_ns,
                    core_source_fingerprint,
                    server_source_fingerprint: args.path.server_source_fingerprint,
                    ext_suffix: args.path.ext_suffix,
                    rustflags: args.path.rustflags,
                    worker_id: args.path.worker_id,
                },
            )?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache seed-canonical",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::ExtensionFresh(args) => {
            let payload =
                current_source_extension_is_fresh_json(&CurrentSourceExtensionFreshnessRequest {
                    metadata_path: args.metadata_path,
                    extension_path: args.extension_path,
                    source_mtime_ns: args.source_mtime_ns,
                    source_fingerprint: args.source_fingerprint,
                });
            emit_current_source_cache_payload(
                "ait-cli current-source-cache extension-fresh",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::BinaryFresh(args) => {
            let payload =
                current_source_binary_is_fresh_json(&CurrentSourceBinaryFreshnessRequest {
                    metadata_path: args.metadata_path,
                    binary_path: args.binary_path,
                    metadata_fingerprint_key: args.metadata_fingerprint_key,
                    metadata_source_mtime_key: args.metadata_source_mtime_key,
                    metadata_mtime_key: args.metadata_mtime_key,
                    metadata_sha_key: args.metadata_sha_key,
                    source_mtime_ns: args.source_mtime_ns,
                    source_fingerprint: args.source_fingerprint,
                });
            emit_current_source_cache_payload(
                "ait-cli current-source-cache binary-fresh",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::CoreFingerprint(args) => {
            let fingerprint = current_core_source_fingerprint_cmd(&args.repo_root)?;
            let payload = json!({
                "repo_root": args.repo_root.to_string_lossy().to_string(),
                "kind": "core",
                "fingerprint": fingerprint,
            });
            emit_current_source_cache_payload(
                "ait-cli current-source-cache core-fingerprint",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::ServerFingerprint(args) => {
            let fingerprint = current_server_source_fingerprint_cmd(&args.repo_root)?;
            let payload = json!({
                "repo_root": args.repo_root.to_string_lossy().to_string(),
                "kind": "server",
                "fingerprint": fingerprint,
            });
            emit_current_source_cache_payload(
                "ait-cli current-source-cache server-fingerprint",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::CoreMtime(args) => {
            let source_mtime_ns = current_core_source_mtime_ns_cmd(&args.repo_root)?;
            let payload = json!({
                "repo_root": args.repo_root.to_string_lossy().to_string(),
                "kind": "core",
                "source_mtime_ns": source_mtime_ns,
            });
            emit_current_source_cache_payload(
                "ait-cli current-source-cache core-mtime",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::ServerMtime(args) => {
            let source_mtime_ns = current_server_source_mtime_ns_cmd(&args.repo_root)?;
            let payload = json!({
                "repo_root": args.repo_root.to_string_lossy().to_string(),
                "kind": "server",
                "source_mtime_ns": source_mtime_ns,
            });
            emit_current_source_cache_payload(
                "ait-cli current-source-cache server-mtime",
                &payload,
                args.json,
            )
        }
    }
}

fn current_source_cache_paths_from_args(
    args: &CurrentSourceCachePathArgs,
) -> Result<ait_core::current_source_cache::CurrentSourceNativeCachePaths, String> {
    let (paths, _, _, _, _) =
        current_source_native_cache_paths(&CurrentSourceNativeCacheRequest {
            namespace_root: args.namespace_root.clone(),
            core_repo_root: args.core_repo_root.clone(),
            core_source_fingerprint: args.core_source_fingerprint.clone(),
            server_source_fingerprint: args.server_source_fingerprint.clone(),
            ext_suffix: args.ext_suffix.clone(),
            rustflags: args.rustflags.clone(),
            worker_id: args.worker_id.clone(),
        })?;
    Ok(paths)
}

use crate::json_support::parse_value;

fn parse_json_object_arg(
    text: &str,
    label: &str,
) -> Result<ait_core::json_support::JsonMap<String, JsonValue>, String> {
    let value = parse_value(text, &format!("{label} must be a JSON object"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} must be a JSON object."))
}

fn emit_current_source_cache_payload(
    title: &str,
    payload: &JsonValue,
    json_output: bool,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "current-source-cache payload must decode to an object.".to_string())?;
    print_key_values(
        title,
        &[
            ("kind", string_field(obj.get("kind"))),
            ("fingerprint", string_field(obj.get("fingerprint"))),
            ("build_key", string_field(obj.get("build_key"))),
            (
                "cache_schema_version",
                string_field(obj.get("cache_schema_version")),
            ),
            ("cache_root", string_field(obj.get("cache_root"))),
            ("target_dir", string_field(obj.get("target_dir"))),
            ("core_repo_root", string_field(obj.get("core_repo_root"))),
            (
                "core_source_fingerprint",
                string_field(obj.get("core_source_fingerprint")),
            ),
            (
                "server_source_fingerprint",
                string_field(obj.get("server_source_fingerprint")),
            ),
            ("worker_id", string_field(obj.get("worker_id"))),
        ],
    );
    Ok(())
}

fn run_blame(repo: RepoRuntime, args: BlameArgs) -> Result<(), String> {
    let _command_range = perfetto_range!("ait.cli.blame.command");
    let payload = {
        let _range = perfetto_range!("ait.cli.blame.compute");
        blame_cmd(
            &repo,
            &BlameRequest {
                path: args.path,
                line: args.line,
                start_line: args.start_line,
                end_line: args.end_line,
                restore: args.restore,
                dry_run: args.dry_run,
                snapshot_id: args.snapshot_id,
                parent_snapshot_id: args.parent_snapshot_id,
                patchset_id: args.patchset_id,
                remote_name: args.remote_name,
                repo_name: args.repo_name,
                change_ref: args.change_ref,
                plan_id: args.plan_id,
                plan_ref: args.plan_ref,
            },
        )?
    };
    {
        let _range = perfetto_range!("ait.cli.blame.render");
        if args.json {
            print_json(&payload)?;
        } else {
            render_human_blame(&payload);
        }
    }
    Ok(())
}

fn run_line(repo: RepoRuntime, command: LineCommand) -> Result<(), String> {
    match command {
        LineCommand::List(args) => {
            let current_line = repo.current_line_name()?;
            let payload = line_list(
                &repo,
                args.include_all,
                args.archived,
                args.remote.as_deref(),
            )?;
            emit_line_list_result(
                &payload,
                args.json,
                args.include_all || args.archived,
                args.remote.as_deref(),
                &current_line,
            )?;
        }
        LineCommand::Create(args) => {
            let payload = line_create(
                &repo,
                &args.name,
                args.from_snapshot.as_deref(),
                args.switch,
                args.restore,
                args.force,
            )?;
            emit_result(
                "ait-cli line create",
                &payload,
                args.json,
                &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "current_line",
                    "switched",
                    "restored",
                ],
            )?;
        }
        LineCommand::Switch(args) => {
            let payload = line_switch(&repo, &args.name, args.restore, args.force)?;
            emit_result(
                "ait-cli line switch",
                &payload,
                args.json,
                &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "current_line",
                ],
            )?;
        }
        LineCommand::Show(args) => {
            let payload = line_show(&repo, args.name.as_deref())?;
            emit_result(
                "ait-cli line show",
                &payload,
                args.json,
                &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "archived_at",
                    "created_at",
                    "updated_at",
                ],
            )?;
        }
        LineCommand::Archive(args) => {
            let payload = line_archive(&repo, &args.name, args.remote.as_deref())?;
            emit_result(
                "ait-cli line archive",
                &payload,
                args.json,
                &[
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "archived_at",
                ],
            )?;
        }
        LineCommand::Rename(args) => {
            let payload = line_rename(&repo, &args.old, &args.new, args.remote.as_deref())?;
            emit_result(
                "ait-cli line rename",
                &payload,
                args.json,
                &[
                    "contract",
                    "operation",
                    "line_id",
                    "old_line_name",
                    "new_line_name",
                    "head_snapshot_id",
                    "remote",
                ],
            )?;
        }
        LineCommand::Delete(args) => {
            let payload = line_delete(&repo, &args.name, args.remote.as_deref(), args.yes)?;
            emit_result(
                "ait-cli line delete",
                &payload,
                args.json,
                &[
                    "contract",
                    "operation",
                    "line_id",
                    "line_name",
                    "status",
                    "head_snapshot_id",
                    "history_preserved_by",
                    "tombstone",
                    "snapshots_deleted",
                    "remote",
                ],
            )?;
        }
        LineCommand::Merge(args) => {
            let payload = line_merge(
                &repo,
                args.source.as_deref(),
                args.target.as_deref(),
                args.message.as_deref(),
                args.continue_merge,
                args.abort_merge,
            )?;
            emit_result(
                "ait-cli line merge",
                &payload,
                args.json,
                &[
                    "status",
                    "target_line_name",
                    "source_line_name",
                    "merge_snapshot_id",
                    "merge_snapshot_created",
                    "conflict_count",
                    "conflict_paths",
                ],
            )?;
        }
        LineCommand::CleanupCandidates(args) => {
            let payload = line_cleanup_candidates(
                &repo,
                Some(args.older_than.as_str()),
                args.cleanup_kind.as_deref(),
                args.include_protected,
            )?;
            emit_line_cleanup_candidates_result(
                &payload,
                args.json,
                args.all,
                args.include_protected,
                &args.older_than,
                args.cleanup_kind.as_deref(),
            )?;
        }
        LineCommand::Cleanup(args) => {
            let payload = line_cleanup(
                &repo,
                Some(args.older_than.as_str()),
                args.cleanup_kind.as_deref(),
                args.limit,
                args.dry_run,
                args.yes,
            )?;
            emit_line_cleanup_report_result(&payload, args.json)?;
        }
    }
    Ok(())
}

fn run_queue(repo: RepoRuntime, command: QueueCommand) -> Result<ExitCode, String> {
    match command {
        QueueCommand::Summary(args) => {
            let _command_range = perfetto_range!("ait.cli.queue.command");
            let payload = {
                let _range = perfetto_range!("ait.cli.queue.compute");
                queue_summary_cmd(
                    &repo,
                    args.remote.as_deref(),
                    &args.status,
                    args.all_changes,
                )?
            };
            {
                let _range = perfetto_range!("ait.cli.queue.render");
                emit_queue_summary_result(&payload, args.json)?;
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn run_remote(repo: RepoRuntime, command: RemoteCommand) -> Result<(), String> {
    match command {
        RemoteCommand::Add(args) => {
            let payload = remote_add_cmd(
                &repo,
                &RemoteAddRequest {
                    name: args.name,
                    url: args.url,
                    repo_name: args.repo_name,
                    make_default: args.default,
                    discard_export: args.discard_export,
                },
            )?;
            emit_remote_add_result(&payload, args.json)?;
        }
        RemoteCommand::List(args) => {
            let payload = remote_list_cmd(&repo)?;
            emit_remote_list_result(&payload, args.json)?;
        }
        RemoteCommand::RecoverHead(_) => {
            unreachable!("remote recover-head is handled before normal repository admission")
        }
    }
    Ok(())
}

fn run_remote_head_recovery(args: RemoteRecoverHeadArgs) -> Result<(), String> {
    let context = RemoteHeadRecoveryContext::discover()?;
    let payload = recover_remote_head(
        &context,
        &RemoteHeadRecoveryRequest {
            remote_name: args.remote,
            line_name: args.line,
            include_line_names: args.include_lines,
            jobs: args.jobs,
            apply: args.apply,
        },
    )?;
    emit_result(
        "ait-cli remote recover-head",
        &payload,
        args.json,
        &[
            "apply",
            "remote",
            "repo_name",
            "line_name",
            "snapshot_id",
            "source_parent_snapshot_id",
            "history_mode",
            "recovered_line_count",
            "recovered_lines",
            "object_pack_count",
            "tree_pack_count",
            "downloaded_object_packs",
            "downloaded_tree_packs",
            "captured_file_count",
            "content_fingerprint",
            "authority_root",
            "pack_root",
            "activation_strategy",
            "single_syscall_atomic",
            "activation_lock_protected",
            "staging_removed",
        ],
    )
}

fn run_release(repo: RepoRuntime, command: ReleaseCommand) -> Result<(), String> {
    let (payload, json_output) = match command {
        ReleaseCommand::Adapter { command } => match command {
            ReleaseAdapterCommand::Check(args) => (
                release_adapter_check_for_target(
                    &repo,
                    &args.version,
                    &args.line_name,
                    args.target.as_deref(),
                )?,
                args.json,
            ),
            ReleaseAdapterCommand::Build(args) => (
                release_adapter_build_for_target(
                    &repo,
                    &args.version,
                    &args.line_name,
                    args.target.as_deref(),
                )?,
                args.json,
            ),
        },
        ReleaseCommand::Candidate { command } => match command {
            ReleaseCandidateCommand::Create(args) => {
                if args.public_source_root.is_some()
                    && args.profile.as_deref() != Some(FAMILY_RELEASE_PROFILE)
                {
                    return Err(
                        "--public-source-root is protected-CI authority input and requires --profile family."
                            .to_string(),
                    );
                }
                let family = args.public_source_root.is_some()
                    || args.profile.as_deref() == Some(FAMILY_RELEASE_PROFILE)
                    || (args.profile.is_none()
                        && family_manifest_exists(&repo, &args.line_name)?);
                let payload = if family {
                    if let Some(public_source_root) = args.public_source_root.as_deref() {
                        family_release_candidate_create_from_public_source(
                            &repo,
                            &args.version,
                            &args.line_name,
                            args.channel.as_deref(),
                            public_source_root,
                        )?
                    } else {
                        family_release_candidate_create(
                            &repo,
                            &args.version,
                            &args.line_name,
                            args.channel.as_deref(),
                        )?
                    }
                } else {
                    if args.channel.is_some() {
                        return Err(
                            "--channel is supported only by a family release candidate. Add ait-release-family.json to the selected Snapshot or pass --profile family."
                                .to_string(),
                        );
                    }
                    release_candidate_create_cmd(
                        &repo,
                        &args.version,
                        &args.line_name,
                        args.profile.as_deref().unwrap_or("local-cli"),
                    )?
                };
                (payload, args.json)
            }
        },
        ReleaseCommand::Check(args) => {
            let payload = if family_candidate_exists(&repo, &args.release_id) {
                if args.tests_command.is_some() || args.skip_tests_reason.is_some() {
                    return Err(
                        "Family release checks consume immutable component receipts; --tests-command and --skip-tests-reason apply only to legacy profiles."
                            .to_string(),
                    );
                }
                let receipts = args.receipts.as_deref().ok_or_else(|| {
                    format!(
                        "Family release {} requires --receipts <dir>.",
                        args.release_id
                    )
                })?;
                family_release_check(
                    &repo,
                    &args.release_id,
                    receipts,
                    args.public_source_root.as_deref(),
                )?
            } else {
                if args.public_source_root.is_some() {
                    return Err(
                        "--public-source-root applies only to a family release candidate."
                            .to_string(),
                    );
                }
                if args.receipts.is_some() {
                    return Err(
                        "--receipts applies only to a family release candidate."
                            .to_string(),
                    );
                }
                release_check_cmd(
                    &repo,
                    &args.release_id,
                    args.tests_command.as_deref(),
                    args.skip_tests_reason.as_deref(),
                )?
            };
            (payload, args.json)
        }
        ReleaseCommand::Build(args) => {
            let payload = if family_candidate_exists(&repo, &args.release_id) {
                if args.native_matrix_dir.is_some() {
                    return Err(
                        "Family release builds consume component receipts; --native-matrix-dir applies only to legacy native profiles."
                            .to_string(),
                    );
                }
                let receipts = args.receipts.as_deref().ok_or_else(|| {
                    format!(
                        "Family release {} requires --receipts <dir>.",
                        args.release_id
                    )
                })?;
                family_release_build(
                    &repo,
                    &args.release_id,
                    receipts,
                    args.public_source_root.as_deref(),
                )?
            } else {
                if args.public_source_root.is_some() {
                    return Err(
                        "--public-source-root applies only to a family release candidate."
                            .to_string(),
                    );
                }
                if args.receipts.is_some() {
                    return Err(
                        "--receipts applies only to a family release candidate."
                            .to_string(),
                    );
                }
                release_build_cmd(
                    &repo,
                    &args.release_id,
                    args.native_matrix_dir.as_deref(),
                )?
            };
            (payload, args.json)
        }
        ReleaseCommand::NativeSource(args) => (
            release_native_source(
                &repo,
                &NativeSourceRequest {
                    release_id: args.release_id,
                    target: args.target,
                    source_dir: args.source_dir,
                    runner: args.runner,
                    runner_image: args.runner_image,
                    rust_toolchain: args.rust_toolchain,
                    rustc_path: args.rustc_path,
                },
            )?,
            args.json,
        ),
        ReleaseCommand::NativeBundle(args) => (
            release_native_bundle_cmd(&repo, &args.release_id, &args.native_matrix_dir)?,
            args.json,
        ),
        ReleaseCommand::Package(args) => (
            family_release_package(&repo, &args.release_id, &args.channel)?,
            args.json,
        ),
        ReleaseCommand::Formula(args) => (
            release_formula_cmd(&repo, &args.release_id, &args.name)?,
            args.json,
        ),
        ReleaseCommand::Show(args) => {
            let payload = if family_candidate_exists(&repo, &args.release_id) {
                if args.remote.is_some() {
                    return Err(
                        "Family release dossiers are local portable evidence; --remote is not supported."
                            .to_string(),
                    );
                }
                family_release_show(&repo, &args.release_id)?
            } else {
                release_show_cmd(&repo, &args.release_id, args.remote.as_deref())?
            };
            (payload, args.json)
        }
        ReleaseCommand::Publish(args) => {
            if family_candidate_exists(&repo, &args.release_id) {
                return Err(family_release_publish_error(&args.release_id));
            }
            (
                release_publish_cmd(&repo, &args.release_id, args.remote.as_deref())?,
                args.json,
            )
        }
        ReleaseCommand::Promote(args) => (
            family_release_promote(&repo, &args.release_id, &args.channel)?,
            args.json,
        ),
    };
    if json_output {
        print_json(&payload)
    } else {
        render_release_text(&payload);
        Ok(())
    }
}

fn run_repo(repo: RepoRuntime, command: RepoCommand) -> Result<(), String> {
    let request = build_repo_command_request(command)?;
    let json_output = request.json_output;
    let payload = repo_command_cmd(&repo, &request)?;
    if json_output {
        print_json(&payload)?;
    } else {
        render_repo_command_text(&request, &payload);
    }
    Ok(())
}

fn run_test(repo: RepoRuntime, command: TestCommand) -> Result<(), String> {
    match command {
        TestCommand::Run(args) => {
            if !args.full {
                return Err(
                    "Native `ait-cli test run` currently supports only `--full`.".to_string(),
                );
            }
            let render_request = repo_request(
                "run-ci",
                args.remote.clone(),
                false,
                JsonMap::new(),
            );
            let payload = test_run_full_cmd(
                &repo,
                &TestRunFullRequest {
                    remote_name: args.remote,
                    json_output: args.json,
                    variant: args.variant,
                    plane: args.plane,
                    target_line: args.target_line,
                    trigger: args.trigger,
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                render_repo_command_text(&render_request, &payload);
            }
        }
        TestCommand::Status(args) => {
            let render_request = repo_request(
                "ci-runs",
                args.remote.clone(),
                false,
                JsonMap::new(),
            );
            let payload = test_status_cmd(
                &repo,
                &TestStatusRequest {
                    remote_name: args.remote,
                    json_output: args.json,
                    plane: args.plane,
                    suite_id: args.suite_id,
                    limit: args.limit,
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                render_repo_command_text(&render_request, &payload);
            }
        }
        TestCommand::PatchsetCi { command } => {
            let (json_output, payload) = match command {
                PatchsetCiSmokeCommand::Preflight(args) => {
                    (args.json, patchset_ci_preflight_cmd(&repo)?)
                }
                PatchsetCiSmokeCommand::PackageSmoke(args) => {
                    (args.json, patchset_ci_package_smoke_cmd(&repo)?)
                }
                PatchsetCiSmokeCommand::StableSmoke(args) => {
                    (args.json, patchset_ci_stable_smoke_cmd(&repo)?)
                }
                PatchsetCiSmokeCommand::ReleaseArtifactSmoke(args) => {
                    (args.json, patchset_ci_release_artifact_smoke_cmd(&repo)?)
                }
                PatchsetCiSmokeCommand::Tg1Required(args) => (
                    args.json,
                    patchset_ci_tg1_required_cmd(&repo, &args.case_ids)?,
                ),
            };
            if json_output {
                print_json(&payload)?;
            } else {
                render_repo_text("patchset-ci-smoke", &payload);
            }
        }
    }
    Ok(())
}

fn build_repo_command_request(command: RepoCommand) -> Result<RepoCommandRequest, String> {
    match command {
        RepoCommand::Show(args) => Ok(repo_request("show", args.remote, args.json, JsonMap::new())),
        RepoCommand::Retire(args) => Ok(repo_request(
            "retire",
            args.remote,
            args.json,
            JsonMap::from_iter([
                ("abort".to_string(), json!(args.abort)),
                (
                    "replace_export".to_string(),
                    json!(args.replace_export),
                ),
            ]),
        )),
        RepoCommand::Restore(args) => Ok(repo_request(
            "restore",
            args.remote,
            args.json,
            JsonMap::new(),
        )),
        RepoCommand::Jobs(args) => {
            let mut data = JsonMap::new();
            data.insert(
                "worker_job_index".to_string(),
                json!(args.worker_job_index),
            );
            data.insert("state".to_string(), json!(args.state));
            data.insert("limit".to_string(), json!(args.limit));
            Ok(repo_request("jobs", args.remote, args.json, data))
        }
        RepoCommand::RunCi(args) => {
            let mut data = JsonMap::new();
            data.insert("suite_ids".to_string(), json!(args.suite_ids));
            data.insert("plane".to_string(), json!(args.plane));
            data.insert("target_line".to_string(), json!(args.target_line));
            data.insert("trigger".to_string(), json!(args.trigger));
            data.insert("selector".to_string(), json!(args.selector));
            data.insert("task_ids".to_string(), json!(args.task_ids));
            data.insert("curated_corpus".to_string(), json!(args.curated_corpus));
            data.insert("count".to_string(), json!(args.count));
            data.insert("window_days".to_string(), json!(args.window_days));
            data.insert(
                "dependency_evidence".to_string(),
                json!(args.dependency_evidence),
            );
            data.insert(
                "compliance_evidence".to_string(),
                json!(args.compliance_evidence),
            );
            Ok(repo_request("run-ci", args.remote, args.json, data))
        }
        RepoCommand::CiCapabilities(args) => Ok(repo_request(
            "ci-capabilities",
            args.remote,
            args.json,
            JsonMap::new(),
        )),
        RepoCommand::CiRuns(args) => {
            let mut data = JsonMap::new();
            data.insert("limit".to_string(), json!(args.limit));
            data.insert("plane".to_string(), json!(args.plane));
            data.insert("suite_id".to_string(), json!(args.suite_id));
            Ok(repo_request("ci-runs", args.remote, args.json, data))
        }
    }
}

fn repo_request(
    command: &str,
    remote_name: Option<String>,
    json_output: bool,
    args: JsonMap<String, JsonValue>,
) -> RepoCommandRequest {
    RepoCommandRequest {
        command: command.to_string(),
        remote_name,
        json_output,
        args,
    }
}
