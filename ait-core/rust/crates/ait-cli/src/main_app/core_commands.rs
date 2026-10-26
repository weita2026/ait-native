fn run_init(args: InitArgs) -> Result<(), String> {
    let payload = crate::binding_services::initialize_repository(&InitRequest {
        root: env::current_dir().map_err(|err| err.to_string())?,
        name: None,
        default_line: "main".to_string(),
        policy_profile: args.policy_profile,
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: args.repair_existing,
    })?;
    if args.json {
        print_json(&payload)?;
    } else {
        println!(
            "{}",
            crate::binding_output::format_initialize_repository(&payload)
        );
    }
    Ok(())
}

fn run_doctor(command: DoctorCommand) -> Result<ExitCode, String> {
    match command {
        DoctorCommand::MemoryRoot(args) => {
            let repo = RepoRuntime::discover()?;
            let payload = doctor_memory_root(&repo)?;
            emit_doctor_result("ait-cli doctor memory-root", &payload, args.json)?;
        }
        DoctorCommand::RuntimeRoot(args) => {
            let repo = RepoRuntime::discover()?;
            let payload = doctor_runtime_root(&repo.authoritative_repo_root())?;
            emit_doctor_result("ait-cli doctor runtime-root", &payload, args.json)?;
        }
        DoctorCommand::PlanAuthority(args) => {
            let repo = RepoRuntime::discover().ok();
            let repo_root = repo.as_ref().map(RepoRuntime::authoritative_repo_root);
            let payload = match repo_root.as_deref() {
                Some(repo_root) => doctor_plan_authority_for_repository(repo_root)?,
                None => doctor_plan_authority()?,
            };
            emit_doctor_result("ait-cli doctor plan-authority", &payload, args.json)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_gc(repo: RepoRuntime, command: GcCommand) -> Result<ExitCode, String> {
    match command {
        GcCommand::Stats(args) => {
            let _command_range = perfetto_range!("ait.cli.gc.stats.command");
            let payload = crate::binding_maintenance::gc_stats(&repo)?;
            let _range = perfetto_range!("ait.cli.gc.stats.render");
            emit_gc_payload("ait-cli gc stats", &payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        GcCommand::Validate(args) => {
            let payload = crate::binding_maintenance::gc_validate(&repo)?;
            let code = crate::binding_maintenance::gc_validate_exit_code(&payload)?;
            emit_gc_payload("ait-cli gc validate", &payload, args.json)?;
            Ok(ExitCode::from(code))
        }
        GcCommand::Prune(args) => {
            let payload = crate::binding_maintenance::gc_prune(&repo, args.apply)?;
            emit_gc_payload("ait-cli gc prune", &payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn emit_gc_payload(title: &str, payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    println!("{}", gc_payload_text(title, payload)?);
    Ok(())
}

pub(crate) fn gc_payload_text(title: &str, payload: &JsonValue) -> Result<String, String> {
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
    Ok(crate::render::key_values_text(
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
            ("mode", string_field(obj.get("mode"))),
            ("applied", string_field(obj.get("applied"))),
            (
                "candidate_orphan_pack_count",
                string_field(obj.get("candidate_orphan_pack_count")),
            ),
            (
                "candidate_orphan_pack_member_count",
                string_field(obj.get("candidate_orphan_pack_member_count")),
            ),
            (
                "candidate_duplicate_blob_count",
                string_field(obj.get("candidate_duplicate_blob_count")),
            ),
            (
                "candidate_verified_fallback_blob_count",
                string_field(obj.get("candidate_verified_fallback_blob_count")),
            ),
            (
                "candidate_base_blob_pointer_rewrite_count",
                string_field(obj.get("candidate_base_blob_pointer_rewrite_count")),
            ),
            (
                "candidate_orphan_pack_ids",
                string_field(obj.get("candidate_orphan_pack_ids")),
            ),
            (
                "candidate_orphan_pack_paths",
                string_field(obj.get("candidate_orphan_pack_paths")),
            ),
            (
                "removed_orphan_pack_count",
                string_field(obj.get("removed_orphan_pack_count")),
            ),
            (
                "removed_orphan_pack_member_count",
                string_field(obj.get("removed_orphan_pack_member_count")),
            ),
            (
                "removed_duplicate_blob_count",
                string_field(obj.get("removed_duplicate_blob_count")),
            ),
            (
                "verified_fallback_blob_count",
                string_field(obj.get("verified_fallback_blob_count")),
            ),
            (
                "rewritten_base_blob_pointer_count",
                string_field(obj.get("rewritten_base_blob_pointer_count")),
            ),
            (
                "removed_orphan_pack_ids",
                string_field(obj.get("removed_orphan_pack_ids")),
            ),
            (
                "removed_orphan_pack_paths",
                string_field(obj.get("removed_orphan_pack_paths")),
            ),
            (
                "already_missing_orphan_pack_paths",
                string_field(obj.get("already_missing_orphan_pack_paths")),
            ),
            (
                "cleanup_warnings",
                string_field(obj.get("cleanup_warnings")),
            ),
            (
                "executed_step_count",
                string_field(obj.get("executed_step_count")),
            ),
        ],
    ))
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
            let payload = crate::binding_source_cache::mark_building(
                &current_source_cache_request_from_args(&args.path),
                args.source_mtime_ns,
                &args.extra_json,
            )?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache mark-building",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::Activate(args) => {
            let payload = crate::binding_source_cache::activate(
                &current_source_cache_request_from_args(&args.path),
                args.source_mtime_ns,
                &args.extra_json,
                args.register_lease,
                args.owner_pid,
            )?;
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
            let payload = crate::binding_source_cache::seed_canonical(
                &current_source_cache_request_from_args(&args.path),
                &args.repo_root,
                &args.canonical_repo_root,
                args.core_source_mtime_ns,
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
            let payload = crate::binding_source_cache::core_fingerprint(&args.repo_root)?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache core-fingerprint",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::ServerFingerprint(args) => {
            let payload = crate::binding_source_cache::server_fingerprint(&args.repo_root)?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache server-fingerprint",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::CoreMtime(args) => {
            let payload = crate::binding_source_cache::core_mtime(&args.repo_root)?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache core-mtime",
                &payload,
                args.json,
            )
        }
        CurrentSourceCacheCommand::ServerMtime(args) => {
            let payload = crate::binding_source_cache::server_mtime(&args.repo_root)?;
            emit_current_source_cache_payload(
                "ait-cli current-source-cache server-mtime",
                &payload,
                args.json,
            )
        }
    }
}

fn current_source_cache_request_from_args(
    args: &CurrentSourceCachePathArgs,
) -> CurrentSourceNativeCacheRequest {
    CurrentSourceNativeCacheRequest {
        namespace_root: args.namespace_root.clone(),
        core_repo_root: args.core_repo_root.clone(),
        core_source_fingerprint: args.core_source_fingerprint.clone(),
        server_source_fingerprint: args.server_source_fingerprint.clone(),
        ext_suffix: args.ext_suffix.clone(),
        rustflags: args.rustflags.clone(),
        worker_id: args.worker_id.clone(),
    }
}

fn emit_current_source_cache_payload(
    title: &str,
    payload: &JsonValue,
    json_output: bool,
) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    println!("{}", current_source_cache_text(title, payload)?);
    Ok(())
}

pub(crate) fn current_source_cache_text(
    title: &str,
    payload: &JsonValue,
) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "current-source-cache payload must decode to an object.".to_string())?;
    Ok(crate::render::key_values_text(
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
    ))
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
                snapshot_id: args.snapshot_id,
                via_parent_snapshot_id: args.via_parent_snapshot_id,
                patchset_id: args.patchset_id,
                remote_name: args.remote_name,
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
            let inventory = crate::binding_services::line_inventory(
                &repo,
                args.include_all,
                args.archived,
                args.remote.as_deref(),
            )?;
            let current_line = string_field(inventory.get("current_line"));
            let payload = &inventory["payload"];
            emit_line_list_result(
                payload,
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
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_line_create(&payload)?);
            }
        }
        LineCommand::Switch(args) => {
            let payload = line_switch(&repo, &args.name, args.restore, args.force)?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_line_switch(&payload)?);
            }
        }
        LineCommand::Show(args) => {
            let payload = if args.remote.is_some() {
                line_show_remote(&repo, args.name.as_deref(), args.remote.as_deref())?
            } else {
                line_show(&repo, args.name.as_deref())?
            };
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_line_show(&payload)?);
            }
        }
        LineCommand::Archive(args) => {
            let payload = line_archive(&repo, &args.name, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_line_archive(&payload)?);
            }
        }
        LineCommand::Rename(args) => {
            let payload = line_rename(&repo, &args.old, &args.new, args.remote.as_deref())?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_line_rename(&payload)?);
            }
        }
        LineCommand::Delete(args) => {
            let payload = line_delete(&repo, &args.name, args.remote.as_deref(), args.yes)?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_line_delete(&payload)?);
            }
        }
        LineCommand::Merge(args) => {
            let payload = line_merge(
                &repo,
                args.source.as_deref(),
                args.message.as_deref(),
                args.continue_merge,
                args.abort_merge,
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_line_merge(&payload)?);
            }
        }
        LineCommand::Cleanup(args) => {
            let payload = line_cleanup(
                &repo,
                Some(args.idle_for.as_str()),
                args.cleanup_kind.as_deref(),
                args.limit,
                args.include_protected,
                args.yes,
            )?;
            emit_line_cleanup_result(
                &payload,
                args.json,
                args.all,
                args.include_protected,
                &args.idle_for,
                args.cleanup_kind.as_deref(),
                args.limit,
            )?;
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
                queue_summary_cmd(&repo, args.remote.as_deref())?
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
                    make_default: args.default,
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
    let root = env::current_dir().map_err(|error| error.to_string())?;
    let payload = ait_cli::binding_services::recover_remote_head(
        &root,
        &ait_cli::remote_head_recovery::RemoteHeadRecoveryRequest {
            remote_name: args.remote,
            jobs: args.jobs,
            apply: args.apply,
        },
    )?;
    if args.json {
        print_json(&payload)
    } else {
        println!(
            "{}",
            ait_cli::binding_output::format_remote_recover_head(&payload)?
        );
        Ok(())
    }
}

fn run_release(repo: RepoRuntime, command: ReleaseCommand) -> Result<(), String> {
    use crate::binding_release as service;
    let (payload, json_output) = match command {
        ReleaseCommand::Adapter { command } => match command {
            ReleaseAdapterCommand::Check(args) => (
                service::check_release_adapter(
                    &repo,
                    &args.version,
                    &args.line_name,
                    args.target.as_deref(),
                )?,
                args.json,
            ),
            ReleaseAdapterCommand::Build(args) => (
                service::build_release_adapter(
                    &repo,
                    &args.version,
                    &args.line_name,
                    args.target.as_deref(),
                )?,
                args.json,
            ),
        },
        ReleaseCommand::Candidate {
            command: ReleaseCandidateCommand::Create(args),
        } => (
            service::create_release_candidate(
                &repo,
                &service::ReleaseCandidateRequest {
                    version: args.version,
                    line_name: args.line_name,
                    profile: args.profile,
                    channel: args.channel,
                    public_source_root: args.public_source_root,
                },
            )?,
            args.json,
        ),
        ReleaseCommand::Check(args) => (
            service::check_release(
                &repo,
                &service::ReleaseCheckRequest {
                    release_id: args.release_id,
                    receipts: args.receipts,
                    public_source_root: args.public_source_root,
                    tests_command: args.tests_command,
                    skip_tests_reason: args.skip_tests_reason,
                },
            )?,
            args.json,
        ),
        ReleaseCommand::Build(args) => (
            service::build_release(
                &repo,
                &service::ReleaseBuildRequest {
                    release_id: args.release_id,
                    receipts: args.receipts,
                    public_source_root: args.public_source_root,
                    native_matrix_dir: args.native_matrix_dir,
                    native_command_dir: args.native_command_dir,
                },
            )?,
            args.json,
        ),
        ReleaseCommand::NativeSource(args) => (
            service::capture_release_native_source(
                &repo,
                &ait_cli::release_surface::NativeSourceRequest {
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
            service::bundle_release_native(&repo, &args.release_id, &args.native_matrix_dir)?,
            args.json,
        ),
        ReleaseCommand::Package(args) => (
            service::package_release(
                &repo,
                &args.release_id,
                &args.channel,
                args.public_source_root.as_deref(),
            )?,
            args.json,
        ),
        ReleaseCommand::Formula(args) => (
            service::generate_release_formula(
                &repo,
                &args.release_id,
                &args.name,
                &args.python_formula,
            )?,
            args.json,
        ),
        ReleaseCommand::Show(args) => (
            service::show_release(
                &repo,
                &service::ReleaseShowRequest {
                    release_id: args.release_id,
                    remote: args.remote,
                    public_source_root: args.public_source_root,
                },
            )?,
            args.json,
        ),
        ReleaseCommand::Publish(args) => (
            service::publish_release(&repo, &args.release_id, args.remote.as_deref())?,
            args.json,
        ),
        ReleaseCommand::Promote(args) => (
            service::promote_release(
                &repo,
                &args.release_id,
                &args.channel,
                args.public_source_root.as_deref(),
            )?,
            args.json,
        ),
    };
    if json_output {
        print_json(&payload)
    } else {
        ait_cli::release_surface::render_release_text(&payload);
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

fn build_repo_command_request(command: RepoCommand) -> Result<RepoCommandRequest, String> {
    match command {
        RepoCommand::Show(args) => Ok(repo_request("show", args.remote, args.json, JsonMap::new())),
        RepoCommand::Retire(args) => Ok(repo_request(
            "retire",
            args.remote,
            args.json,
            JsonMap::from_iter([("abort".to_string(), json!(args.abort))]),
        )),
        RepoCommand::Restore(args) => Ok(repo_request(
            "restore",
            args.remote,
            args.json,
            JsonMap::new(),
        )),
        RepoCommand::Jobs(args) => {
            let mut data = JsonMap::new();
            if let Some(worker_job_index) = args.worker_job_index {
                data.insert("worker_job_index".to_string(), json!(worker_job_index));
            } else {
                data.insert("state".to_string(), json!(args.state));
                data.insert("limit".to_string(), json!(args.limit));
            }
            Ok(repo_request("jobs", args.remote, args.json, data))
        }
        RepoCommand::CiCapabilities(args) => Ok(repo_request(
            "ci-capabilities",
            args.remote,
            args.json,
            JsonMap::new(),
        )),
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
