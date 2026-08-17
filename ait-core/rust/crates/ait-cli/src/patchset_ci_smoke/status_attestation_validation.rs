use super::*;

pub(super) fn run_tg1_check(
    check_id: &str,
    repo: &RepoRuntime,
    root: &Path,
    source_root: Option<&Path>,
) -> Result<(), String> {
    match check_id {
        "plan_public_surface" => assert_public_plan_contract(root),
        "plan_source_guard" => {
            assert_plan_source_files_omit_legacy_line_alignment_contract(root, source_root)
        }
        "plan_sync_lineage_only" => assert_plan_sync_stays_lineage_only(),
        "root_worktree_plan_sync_guard" => assert_plan_sync_bypasses_root_worktree_guard(),
        "init_sprint_readme_guard" => assert_init_establishes_agent_contract(),
        "sprint_readme_contract" => assert_sprint_readme_contract(root),
        "stable_remote_land_flow" => {
            super::suite_execution::run_stable_smoke_inner(repo)?;
            Ok(())
        }
        "final_snapshot_remote_promotion_contract" => {
            assert_final_snapshot_remote_promotion_contract()
        }
        other => Err(format!("Unknown TG1 Rust check `{other}`.")),
    }
}

pub(super) fn body_mentions_pass_tests(body: &str) -> bool {
    let Some(payload) = parse_value_option(body) else {
        return body.contains("\"tests\"") && body.contains("pass");
    };
    matches!(
        payload.get("evaluation_summary").and_then(JsonValue::as_object),
        Some(evaluation_summary) if evaluation_summary.get("tests").and_then(JsonValue::as_str) == Some("pass")
    ) || payload.get("tests").and_then(JsonValue::as_str) == Some("pass")
}

pub(super) fn is_attestation_request(url: &str) -> bool {
    url.starts_with("/v1/native/repository-authorities/7/patchsets/")
        && (url.ends_with(":attestation") || url.ends_with("/attestation"))
}

pub(super) fn workspace_root_for_scan(repo_root: &Path) -> PathBuf {
    repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf())
}

pub(super) fn iter_markdown_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let normalized_root = workspace_root_for_scan(root);
    let mut files = Vec::new();
    collect_markdown_files(&normalized_root, &normalized_root, &mut files)?;
    files.sort();
    Ok(files)
}

pub(super) fn collect_markdown_files(
    root: &Path,
    cursor: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(cursor).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let file_type = entry.file_type().map_err(|err| err.to_string())?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .map_err(|err| err.to_string())?
            .to_path_buf();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if should_ignore_part(name) {
                continue;
            }
            collect_markdown_files(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() || path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        if rel.components().count() == 1 && name.starts_with("codex-") {
            continue;
        }
        if rel
            .components()
            .any(|component| should_ignore_part(component.as_os_str().to_string_lossy().as_ref()))
        {
            continue;
        }
        files.push(path);
    }
    Ok(())
}

pub(super) fn should_ignore_part(part: &str) -> bool {
    IGNORED_DIRS.contains(&part) || part.starts_with(".tmp-")
}

pub(super) fn find_broken_links(root: &Path) -> Result<Vec<LinkIssue>, String> {
    let normalized_root = workspace_root_for_scan(root);
    let mut issues = Vec::new();
    for path in iter_markdown_files(&normalized_root)? {
        for (line_number, line) in iter_scan_lines(&path)? {
            for destination in line_destinations(&line) {
                let Some(target) = normalize_local_target(&destination) else {
                    continue;
                };
                let resolved_path = resolve_target(&normalized_root, &path, &target);
                let fallback_path =
                    resolve_source_root_target(&normalized_root, &path, &target, &resolved_path)?;
                let exists = resolved_path.exists()
                    || fallback_path
                        .as_ref()
                        .map(|candidate| candidate.exists())
                        .unwrap_or(false);
                if exists || should_skip_missing_target(&normalized_root, &path, &resolved_path) {
                    continue;
                }
                issues.push(LinkIssue {
                    path: path
                        .strip_prefix(&normalized_root)
                        .map_err(|err| err.to_string())?
                        .to_path_buf(),
                    line_number,
                    target: destination,
                    resolved_path,
                });
            }
        }
    }
    Ok(issues)
}

pub(super) fn iter_scan_lines(path: &Path) -> Result<Vec<(usize, String)>, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut lines = Vec::new();
    let mut in_fence = false;
    for (index, raw_line) in text.lines().enumerate() {
        if is_fence_line(raw_line) {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            lines.push((index + 1, raw_line.to_string()));
        }
    }
    Ok(lines)
}

pub(super) fn is_fence_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let leading_spaces = line.len() - trimmed.len();
    leading_spaces <= 3 && (trimmed.starts_with("```") || trimmed.starts_with("~~~"))
}

pub(super) fn line_destinations(line: &str) -> Vec<String> {
    let mut items = inline_link_destinations(line);
    if let Some(reference) = reference_definition_destination(line) {
        items.push(reference);
    }
    items
}

pub(super) fn inline_link_destinations(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut items = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'[' {
            index += 1;
            continue;
        }
        let mut close = index + 1;
        let mut found = None;
        while close + 1 < bytes.len() {
            if bytes[close] == b']' && bytes[close + 1] == b'(' {
                found = Some(close);
                break;
            }
            close += 1;
        }
        let Some(close) = found else {
            index += 1;
            continue;
        };
        let start = close + 2;
        let mut end = start;
        let mut depth = 0usize;
        while end < bytes.len() {
            match bytes[end] {
                b'(' => depth += 1,
                b')' if depth == 0 => break,
                b')' => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        if end < bytes.len() && end > start {
            items.push(extract_destination(&line[start..end]));
            index = end + 1;
            continue;
        }
        index += 1;
    }
    items
}

pub(super) fn reference_definition_destination(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if line.len() - trimmed.len() > 3 || !trimmed.starts_with('[') {
        return None;
    }
    let label_end = trimmed.find("]:")?;
    let rest = trimmed.get(label_end + 2..)?.trim_start();
    if rest.is_empty() {
        return None;
    }
    Some(extract_destination(rest))
}

pub(super) fn extract_destination(raw: &str) -> String {
    let text = raw.trim();
    if let Some(rest) = text.strip_prefix('<') {
        if let Some(end) = rest.find('>') {
            return rest[..end].trim().to_string();
        }
    }
    text.split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

pub(super) fn normalize_local_target(destination: &str) -> Option<String> {
    let target = destination.trim();
    if target.is_empty()
        || target.starts_with('#')
        || target.starts_with("//")
        || has_scheme_prefix(target)
    {
        return None;
    }
    let path = target.split('#').next().unwrap_or("").trim();
    if path.is_empty() {
        return None;
    }
    Some(path.to_string())
}

pub(super) fn has_scheme_prefix(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    for ch in chars {
        if ch == ':' {
            return true;
        }
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-')) {
            return false;
        }
    }
    false
}

pub(super) fn resolve_target(root: &Path, source: &Path, target: &str) -> PathBuf {
    if target.starts_with('/') {
        normalize_path(root.join(target.trim_start_matches('/')))
    } else {
        normalize_path(source.parent().unwrap_or(root).join(target))
    }
}

pub(super) fn normalize_path(path: PathBuf) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

pub(super) fn resolve_source_root_target(
    root: &Path,
    source: &Path,
    target: &str,
    resolved_path: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(source_root) = source_root_for_workspace(root)? else {
        return Ok(None);
    };
    if let Ok(target_rel) = resolved_path.strip_prefix(root) {
        return Ok(Some(normalize_path(source_root.join(target_rel))));
    }
    let Ok(source_rel) = source.strip_prefix(root) else {
        return Ok(None);
    };
    let authored_source = normalize_path(source_root.join(source_rel));
    Ok(Some(normalize_path(
        authored_source
            .parent()
            .unwrap_or(&source_root)
            .join(target),
    )))
}

pub(super) fn source_root_for_workspace(root: &Path) -> Result<Option<PathBuf>, String> {
    let metadata_path = root.join(".ait-worktree.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&metadata_path).map_err(|err| err.to_string())?;
    let payload = parse_value_error_string(&text)?;
    let Some(repo_root) = payload.get("repo_root").and_then(JsonValue::as_str) else {
        return Ok(None);
    };
    if repo_root.trim().is_empty() {
        return Ok(None);
    }
    let candidate = workspace_root_for_scan(Path::new(repo_root));
    if candidate == root {
        return Ok(None);
    }
    Ok(Some(candidate))
}

pub(super) fn should_skip_missing_target(root: &Path, source: &Path, resolved_path: &Path) -> bool {
    let Ok(target_rel) = resolved_path.strip_prefix(root) else {
        return false;
    };
    let Ok(source_rel) = source.strip_prefix(root) else {
        return false;
    };
    let target_parts = target_rel
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let source_parts = source_rel
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if !root.join("docs/plan.md").exists()
        && target_parts.first().map(|value| value.as_str()) == Some("docs")
        && source_parts.first().map(|value| value.as_str()) != Some("docs")
    {
        return true;
    }
    if !root.join("docs/sprints").exists()
        && target_parts.first().map(|value| value.as_str()) == Some("docs")
        && target_parts.get(1).map(|value| value.as_str()) == Some("sprints")
    {
        return true;
    }
    if source_parts.first().map(|value| value.as_str()) == Some("release")
        && source_parts.get(1).map(|value| value.as_str()) == Some("guides")
        && resolved_path.extension().and_then(|value| value.to_str()) == Some("md")
    {
        if target_parts.len() == 1 && !root.join("README.md").exists() {
            return true;
        }
        if target_parts.first().map(|value| value.as_str()) == Some("release")
            && target_parts.get(1).map(|value| value.as_str()) != Some("guides")
            && target_parts
                .get(1)
                .map(|value| !root.join("release").join(value).exists())
                .unwrap_or(false)
        {
            return true;
        }
    }
    if source_parts.first().map(|value| value.as_str()) != Some("docs")
        || source_parts.get(1).map(|value| value.as_str()) != Some("sprints")
    {
        return false;
    }
    if !root.join("docs/plan.md").exists() {
        return true;
    }
    if resolved_path.extension().and_then(|value| value.to_str()) != Some("md") {
        return false;
    }
    !(target_parts.first().map(|value| value.as_str()) == Some("docs")
        && target_parts.get(1).map(|value| value.as_str()) == Some("sprints"))
}

pub(super) fn format_broken_links(issues: &[LinkIssue]) -> String {
    let mut lines = vec!["Broken local Markdown links:".to_string()];
    for issue in issues {
        lines.push(format!(
            "{}:{}: {:?} -> {}",
            issue.path.display(),
            issue.line_number,
            issue.target,
            issue.resolved_path.display()
        ));
    }
    lines.join("\n")
}

pub(super) fn assert_public_plan_contract(repo_root: &Path) -> Result<(), String> {
    let plan_help = command_output(repo_root, &["plan", "--help"])?;
    let (_, commands) = help_inventory(&plan_help.stdout);
    let (sync_options, _) =
        help_inventory(&command_output(repo_root, &["plan", "sync", "--help"])?.stdout);
    if commands.contains("create") || commands.contains("revise") {
        return Err("plan help still exposes create/revise".to_string());
    }
    if sync_options.contains("--default-line") {
        return Err("plan sync help still exposes --default-line".to_string());
    }
    for forbidden in [
        "line_sync",
        "root_main_sync",
        "remote_main_sync",
        "--default-line",
    ] {
        if plan_help.stdout.contains(forbidden) {
            return Err(format!(
                "plan help still contains forbidden token `{forbidden}`"
            ));
        }
    }
    for subcommand in ["create", "revise"] {
        let output = command_output(repo_root, &["plan", subcommand])?;
        if output.status == 0 || !combined_output(&output).contains("unrecognized subcommand") {
            return Err(format!(
                "plan {subcommand} no longer fails as an unrecognized subcommand"
            ));
        }
    }
    let default_line = command_output(
        repo_root,
        &["plan", "sync", "README.md", "--default-line", "main"],
    )?;
    if default_line.status == 0
        || !combined_output(&default_line).contains("unexpected argument '--default-line'")
    {
        return Err(
            "plan sync --default-line no longer fails with the expected contract".to_string(),
        );
    }
    Ok(())
}

pub(super) fn assert_release_python_authority_retired(repo_root: &Path) -> Result<(), String> {
    for relative_path in RETIRED_RELEASE_PYTHON_PATHS {
        if repo_root.join(relative_path).exists() {
            return Err(format!(
                "retired Python release authority returned at {relative_path}"
            ));
        }
    }

    let pyproject_path = repo_root.join("pyproject.toml");
    let pyproject = fs::read_to_string(&pyproject_path)
        .map_err(|err| format!("failed to read {}: {err}", pyproject_path.display()))?;
    if !pyproject.contains("ait = \"ait.cli_entrypoint:main\"") {
        return Err(
            "public `ait` console script does not use the pre-import native exec entrypoint"
                .to_string(),
        );
    }

    let entrypoint_path = repo_root.join("src/ait/cli_entrypoint.py");
    let entrypoint = fs::read_to_string(&entrypoint_path).map_err(|err| {
        format!(
            "failed to read native release console entrypoint {}: {err}",
            entrypoint_path.display()
        )
    })?;
    let exec_offset = entrypoint.find("os.execvpe(").ok_or_else(|| {
        "public `ait release` console entrypoint does not process-replace Python with Rust"
            .to_string()
    })?;
    let python_app_offset = entrypoint.find("from .cli import app").ok_or_else(|| {
        "public `ait` console entrypoint does not preserve the non-native Python CLI path"
            .to_string()
    })?;
    if !entrypoint.contains("\"release\"")
        || entrypoint.contains("subprocess")
        || exec_offset > python_app_offset
    {
        return Err(
            "public `ait release` does not exec Rust before loading the Python CLI application"
                .to_string(),
        );
    }

    let app_surfaces_path = repo_root.join("src/ait/cli/app_surfaces.py");
    let app_surfaces = fs::read_to_string(&app_surfaces_path).map_err(|err| {
        format!(
            "failed to read Python CLI surface registration {}: {err}",
            app_surfaces_path.display()
        )
    })?;
    if app_surfaces.contains("\"release\"") || app_surfaces.contains("release_app") {
        return Err(
            "public `ait release` is still registered in the Python CLI application".to_string(),
        );
    }

    let namespace_path = repo_root.join("src/ait/cli/native_namespace_command.py");
    let namespace = fs::read_to_string(&namespace_path).map_err(|err| {
        format!(
            "failed to read native namespace gate {}: {err}",
            namespace_path.display()
        )
    })?;
    if namespace.contains("\"release\"") {
        return Err("returning Python native namespace gate still includes `release`".to_string());
    }

    let bootstrap_path = repo_root.join("src/ait/cli/commands/bootstrap.py");
    let bootstrap = fs::read_to_string(&bootstrap_path).map_err(|err| {
        format!(
            "failed to read Python command bootstrap {}: {err}",
            bootstrap_path.display()
        )
    })?;
    if bootstrap.contains("\"release\"") {
        return Err("Python command bootstrap still routes `release`".to_string());
    }

    let manifest_path = repo_root.join("ci/patch_ci.json");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    if manifest.contains("patchset ci-smoke") {
        return Err(
            "patch CI still routes through the retired public `ait patchset ci-smoke` command"
                .to_string(),
        );
    }
    let manifest_payload = parse_value_error_string(&manifest)
        .map_err(|error| format!("Invalid {}: {error}", manifest_path.display()))?;
    let uses_internal_runner = manifest_payload
        .get("suites")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|suite| suite.get("runner"))
        .any(|runner| {
            runner.get("kind").and_then(JsonValue::as_str) == Some("test_discovery_sharded")
                && runner
                    .get("build_args")
                    .and_then(JsonValue::as_array)
                    .is_some_and(|args| {
                        args.iter()
                            .any(|arg| arg.as_str() == Some("patchset_ci_runner"))
                    })
        });
    if !uses_internal_runner {
        return Err(
            "patch CI does not include the internal Rust patchset_ci_runner test target"
                .to_string(),
        );
    }
    for forbidden in [
        "ait release candidate",
        "ait release check",
        "ait release build",
        "python -m ait",
    ] {
        if manifest.contains(forbidden) {
            return Err(format!(
                "release_artifact_smoke still contains Python release routing token `{forbidden}`"
            ));
        }
    }
    Ok(())
}

pub(super) fn assert_plan_sync_stays_lineage_only() -> Result<(), String> {
    let temp = TempDir::new().map_err(|err| err.to_string())?;
    let repo = temp.path().join("repo");
    fs::create_dir_all(repo.join("docs/sprints")).map_err(|err| err.to_string())?;
    fs::write(repo.join("README.md"), "base\n").map_err(|err| err.to_string())?;
    fs::write(
        repo.join("docs/sprints/contract.md"),
        "# Contract\n\n## Keep Sync Lineage Only [plan-ref: contract/root]\n\n- [ ] Keep sync lineage only [ref: contract/lineage-only]\n",
    )
    .map_err(|err| err.to_string())?;

    json_output(&repo, &["init", "--json"])?;
    json_output(
        &repo,
        &["snapshot", "create", "--message", "seed", "--json"],
    )?;
    let line_before = json_output(&repo, &["line", "show", "main", "--json"])?;
    let sync_help = command_output(&repo, &["plan", "sync", "--help"])?;
    let sync_payload = json_output(
        &repo,
        &["plan", "sync", "docs/sprints/contract.md", "--json"],
    )?;
    let line_after = json_output(&repo, &["line", "show", "main", "--json"])?;

    for forbidden in [
        "line_sync",
        "root_main_sync",
        "remote_main_sync",
        "--default-line",
    ] {
        if sync_help.stdout.contains(forbidden) {
            return Err(format!(
                "plan sync help still contains forbidden token `{forbidden}`"
            ));
        }
        if sync_payload.get(forbidden).is_some() {
            return Err(format!("plan sync payload still exposes `{forbidden}`"));
        }
    }
    if string_field(&line_after, "head_snapshot_id")
        != string_field(&line_before, "head_snapshot_id")
    {
        return Err("plan sync moved the line head instead of staying lineage-only".to_string());
    }
    Ok(())
}

pub(super) fn assert_plan_sync_bypasses_root_worktree_guard() -> Result<(), String> {
    let temp = TempDir::new().map_err(|err| err.to_string())?;
    let repo = temp.path().join("repo");
    fs::create_dir_all(repo.join("docs/sprints")).map_err(|err| err.to_string())?;
    fs::write(repo.join("README.md"), "base\n").map_err(|err| err.to_string())?;
    fs::write(
        repo.join("docs/sprints/root_guard.md"),
        "# Root Guard\n\n## Keep Public Plan Sync Available [plan-ref: contract/root-guard]\n\n- [ ] keep repo-root plan sync unblocked [ref: contract/root-guard-bypass]\n",
    )
    .map_err(|err| err.to_string())?;

    json_output(&repo, &["init", "--json"])?;
    json_output(
        &repo,
        &["snapshot", "create", "--message", "seed", "--json"],
    )?;
    json_output(&repo, &["config", "set", "--sprint", "off", "--json"])?;
    let task_id = "RT-ROOT-GUARD";
    let worktree_name = "rt-root-guard";
    let worktree_path = repo.join("task-worktrees").join(worktree_name);
    let config_path = repo.join(".ait/config.json");
    let mut config = parse_value_error_string(
        &fs::read_to_string(&config_path).map_err(|err| err.to_string())?,
    )?;
    config["worktree_name"] = JsonValue::String(worktree_name.to_string());
    fs::write(
        &config_path,
        format!("{}\n", encode_value_or(&config, "{}")),
    )
    .map_err(|err| err.to_string())?;
    let registry_dir = repo.join(".ait/worktrees");
    fs::create_dir_all(&registry_dir).map_err(|err| err.to_string())?;
    fs::write(
        registry_dir.join(format!("{worktree_name}.json")),
        format!(
            "{}\n",
            encode_value_or(
                &json!({
                    "name": worktree_name,
                    "path": worktree_path.to_string_lossy(),
                    "repo_root": repo.to_string_lossy(),
                    "line_name": "feature/rt-root-guard",
                    "bound_task_id": task_id,
                    "bound_change_id": "RC-ROOT-GUARD",
                    "auto_created_for_task": true,
                }),
                "{}",
            )
        ),
    )
    .map_err(|err| err.to_string())?;
    let blocked_snapshot = command_output(
        &repo,
        &["snapshot", "create", "--message", "blocked from root"],
    )?;
    let blocked_output = combined_output(&blocked_snapshot);
    if blocked_snapshot.status == 0
        || !blocked_output.contains("Repo root is pinned to bound worktree")
    {
        return Err("snapshot create no longer trips the root worktree guard".to_string());
    }
    if !blocked_output.contains(task_id) || !blocked_output.contains(worktree_name) {
        return Err("root worktree guard no longer reports the bound task/worktree".to_string());
    }
    let sync_out = command_output(
        &repo,
        &["plan", "sync", "docs/sprints/root_guard.md", "--json"],
    )?;
    if sync_out.status != 0 {
        return Err(format!(
            "plan sync failed under root worktree guard: {}",
            combined_output(&sync_out)
        ));
    }
    if combined_output(&sync_out).contains("Repo root is pinned to bound worktree") {
        return Err("plan sync still trips the root worktree guard".to_string());
    }
    Ok(())
}

pub(super) fn assert_plan_source_files_omit_legacy_line_alignment_contract(
    repo_root: &Path,
    source_root: Option<&Path>,
) -> Result<(), String> {
    for (path, forbidden_tokens) in PLAN_SOURCE_TOKEN_FORBIDDEN {
        let text = read_tg1_source_guard_file(repo_root, path, source_root)?;
        for forbidden in *forbidden_tokens {
            if text.contains(forbidden) {
                return Err(format!(
                    "{path} still contains forbidden plan token: {forbidden}"
                ));
            }
        }
    }
    for (path, patterns) in PLAN_SOURCE_REGEX_FORBIDDEN {
        let text = read_tg1_source_guard_file(repo_root, path, source_root)?;
        for pattern in *patterns {
            let Some((left, right)) = pattern.split_once("::{0,400}") else {
                continue;
            };
            if contains_ordered_tokens(&text, left, right, 400) {
                return Err(format!(
                    "{path} still matches forbidden plan pattern: {pattern}"
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn read_tg1_source_guard_file(
    repo_root: &Path,
    relative_path: &str,
    source_root: Option<&Path>,
) -> Result<String, String> {
    read_tg1_source_guard_file_with_fallback(repo_root, relative_path, source_root)
}

pub(super) fn read_tg1_source_guard_file_with_fallback(
    repo_root: &Path,
    relative_path: &str,
    target_root: Option<&Path>,
) -> Result<String, String> {
    let primary = repo_root.join(relative_path);
    match fs::read_to_string(&primary) {
        Ok(text) => Ok(text),
        Err(primary_error) => {
            if let Some(target_root) = target_root {
                let fallback = target_root.join(relative_path);
                if fallback != primary && fallback.is_file() {
                    return fs::read_to_string(&fallback).map_err(|fallback_error| {
                        format!(
                            "Failed to read TG1 source guard fallback `{}` after `{}` failed: {}; fallback error: {}",
                            fallback.display(),
                            primary.display(),
                            primary_error,
                            fallback_error
                        )
                    });
                }
            }
            Err(format!(
                "Failed to read TG1 source guard file `{}`: {}",
                primary.display(),
                primary_error
            ))
        }
    }
}

pub(super) fn contains_ordered_tokens(text: &str, left: &str, right: &str, max_gap: usize) -> bool {
    let mut search_from = 0usize;
    while let Some(left_index) = text[search_from..].find(left) {
        let left_start = search_from + left_index;
        let after_left = left_start + left.len();
        let end = after_left.saturating_add(max_gap).min(text.len());
        if text[after_left..end].contains(right) {
            return true;
        }
        search_from = after_left;
    }
    false
}

pub(super) fn assert_init_establishes_agent_contract() -> Result<(), String> {
    let temp = TempDir::new().map_err(|err| err.to_string())?;
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).map_err(|err| err.to_string())?;
    let init_help = command_output(&repo, &["init", "--help"])?;
    let (init_options, _) = help_inventory(&init_help.stdout);
    let expected_options = ["--policy-profile", "--repair-existing", "--json", "--help"]
        .into_iter()
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if init_options != expected_options {
        return Err(format!(
            "init option inventory differs from the deterministic bootstrap contract: {:?}",
            init_options
        ));
    }
    let init_payload = json_output(&repo, &["init", "--json"])?;
    if init_payload["action"].as_str() != Some("initialized") {
        return Err("init did not report the initialized action".to_string());
    }
    for removed_field in [
        "agent_harness",
        "bootstrap_files",
        "bootstrap_guide",
        "bootstrap_directories",
        "forbidden_bootstrap_paths",
        "next_steps",
    ] {
        if init_payload.get(removed_field).is_some() {
            return Err(format!(
                "init unexpectedly returned onboarding field `{removed_field}`"
            ));
        }
    }
    let agents = fs::read_to_string(repo.join("AGENTS.md")).map_err(|err| err.to_string())?;
    if !agents.contains("<!-- ait:workflow:start -->") || !agents.contains("sprint mode: `on`") {
        return Err("init did not create the effective AGENTS.md contract".to_string());
    }
    if !repo.join("docs/sprints").is_dir() {
        return Err("sprint-on init did not create docs/sprints/".to_string());
    }
    if repo.join("docs/sprints/README.md").exists() {
        return Err("init unexpectedly created a sprint placeholder".to_string());
    }
    for forbidden_path in ["ait-native.md", "docs/plan.md", "docs/milestone.md"] {
        if repo.join(forbidden_path).exists() {
            return Err(format!(
                "init unexpectedly materialized legacy bootstrap path `{forbidden_path}`"
            ));
        }
    }
    Ok(())
}

pub(super) fn assert_sprint_readme_contract(repo_root: &Path) -> Result<(), String> {
    let sprint_readme = repo_root.join("docs/sprints/README.md");
    if !sprint_readme.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&sprint_readme).map_err(|err| err.to_string())?;
    for forbidden in ["ait plan create", "ait plan revise", "plan create|revise"] {
        if text.contains(forbidden) {
            return Err(format!(
                "docs/sprints/README.md still contains `{forbidden}`"
            ));
        }
    }
    for line in text.lines() {
        let trimmed = line.trim_start();
        if (trimmed.starts_with('#') || trimmed.starts_with("- ["))
            && line.contains("[plan-ref:")
            && !line.contains("`[plan-ref:")
        {
            return Err("docs/sprints/README.md still exposes a raw plan-ref".to_string());
        }
        if trimmed.starts_with("- [") && line.contains("[ref:") && !line.contains("`[ref:") {
            return Err("docs/sprints/README.md still exposes a raw checklist ref".to_string());
        }
    }
    for required in [
        "directory note only",
        "should not become",
        "primary entry surface",
    ] {
        if !text.contains(required) {
            return Err(format!(
                "docs/sprints/README.md is missing required phrase `{required}`"
            ));
        }
    }
    let routing_text = fs::read_to_string(repo_root.join("docs/sprint_artifact_routing.md"))
        .map_err(|err| err.to_string())?;
    if !routing_text.contains("Do not treat `docs/sprints/README.md`")
        || !routing_text.contains("authority layer")
    {
        return Err(
            "docs/sprint_artifact_routing.md no longer guards sprint README authority".to_string(),
        );
    }
    let quickstart_text = fs::read_to_string(repo_root.join("docs/ait_native_quickstart.md"))
        .map_err(|err| err.to_string())?;
    for required in [
        "must not create",
        "docs/sprints/README.md",
        "sprint entry surface",
    ] {
        if !quickstart_text.contains(required) {
            return Err(format!(
                "docs/ait_native_quickstart.md is missing `{required}`"
            ));
        }
    }
    Ok(())
}

pub(super) fn command_output(repo_root: &Path, args: &[&str]) -> Result<CommandOutput, String> {
    let exe = internal_cli_executable()?;
    let output = Command::new(exe)
        .current_dir(repo_root)
        .args(args)
        .output()
        .map_err(|err| err.to_string())?;
    Ok(CommandOutput {
        status: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub(super) fn json_output(repo_root: &Path, args: &[&str]) -> Result<JsonValue, String> {
    let output = command_output(repo_root, args)?;
    if output.status != 0 {
        return Err(format!(
            "command failed: ait-cli {}\n{}",
            args.join(" "),
            combined_output(&output)
        ));
    }
    parse_value_error_string(&output.stdout)
}

pub(super) fn combined_output(output: &CommandOutput) -> String {
    match (output.stdout.trim(), output.stderr.trim()) {
        ("", "") => String::new(),
        ("", stderr) => stderr.to_string(),
        (stdout, "") => stdout.to_string(),
        (stdout, stderr) => format!("{stdout}\n{stderr}"),
    }
}

pub(super) fn help_inventory(help_text: &str) -> (HashSet<String>, HashSet<String>) {
    let mut options = HashSet::new();
    let mut commands = HashSet::new();
    let mut section = "";
    for raw_line in help_text.lines() {
        let line = raw_line.trim_end();
        if line == "Commands:" {
            section = "commands";
            continue;
        }
        if line == "Options:" {
            section = "options";
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        match section {
            "commands" => {
                if let Some(token) = trimmed.split_whitespace().next() {
                    if token
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch == '-' || ch.is_ascii_digit())
                    {
                        commands.insert(token.to_string());
                    }
                }
            }
            "options" => {
                for token in trimmed.split(|ch: char| ch.is_whitespace() || ch == ',') {
                    if token.starts_with("--") && token.len() > 2 {
                        options.insert(token.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    (options, commands)
}

#[cfg(test)]
mod tg1_source_guard_tests {
    use super::*;

    #[test]
    fn source_guard_file_falls_back_to_target_root() {
        let temp = TempDir::new().unwrap();
        let inner_root = temp.path().join("inner");
        let target_root = temp.path().join("target");
        fs::create_dir_all(target_root.join("src/ait/cli")).unwrap();
        fs::write(
            target_root.join("src/ait/cli/app.py"),
            "def main():\n    pass\n",
        )
        .unwrap();

        let text = read_tg1_source_guard_file_with_fallback(
            &inner_root,
            "src/ait/cli/app.py",
            Some(&target_root),
        )
        .unwrap();

        assert!(text.contains("def main"));
    }
}
