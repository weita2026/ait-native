fn run_git_command(repo: RepoRuntime, command: GitCommand) -> Result<(), String> {
    match command {
        GitCommand::Import(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli git import", || {
                git_import_cmd(
                    &repo,
                    &args.source,
                    args.all_branches_and_tags,
                    args.dry_run,
                )
            })?;
            emit_result(
                "ait-cli git import",
                &payload,
                args.json,
                &[
                    "status",
                    "operation_id",
                    "generation_id",
                    "source_repository_fingerprint",
                    "git_object_format",
                    "commit_count",
                    "imported_commit_count",
                    "reused_commit_count",
                    "line_count",
                    "tag_count",
                    "mutated",
                ],
            )
        }
        GitCommand::Export(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli git export", || {
                git_export_cmd(
                    &repo,
                    &args.target,
                    args.all_lines_and_tags,
                    args.dry_run,
                )
            })?;
            emit_result(
                "ait-cli git export",
                &payload,
                args.json,
                &[
                    "status",
                    "operation_id",
                    "generation_id",
                    "target_repository_fingerprint",
                    "git_object_format",
                    "snapshot_count",
                    "exact_git_object_reuse_count",
                    "native_commit_count",
                    "ref_count",
                    "compare_and_swap",
                    "force_updated",
                    "fsck",
                    "mutated",
                ],
            )
        }
        GitCommand::Mirror(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli git mirror", || {
                git_mirror_cmd(
                    &repo,
                    &args.endpoint,
                    &args.direction,
                    args.dry_run,
                )
            })?;
            emit_result(
                "ait-cli git mirror",
                &payload,
                args.json,
                &[
                    "status",
                    "operation_id",
                    "generation_id",
                    "direction",
                    "endpoint_repository_fingerprint",
                    "state",
                    "equal_count",
                    "inbound_only_count",
                    "outbound_only_count",
                    "divergent_count",
                    "compare_and_swap",
                    "force_updated",
                    "mutated",
                ],
            )
        }
    }
}
