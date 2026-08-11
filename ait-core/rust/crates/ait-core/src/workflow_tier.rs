use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const WORKFLOW_TIER_CONTRACT: &str = "ait.workflow-tier/v1";
pub const DEFAULT_QUICK_MAX_FILES: usize = 8;
pub const DEFAULT_QUICK_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTier {
    QuickModification,
    NormalTask,
    FullyGoverned,
}

impl WorkflowTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QuickModification => "quick_modification",
            Self::NormalTask => "normal_task",
            Self::FullyGoverned => "fully_governed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTierLimits {
    pub max_files: usize,
    pub max_bytes: u64,
}

impl Default for WorkflowTierLimits {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_QUICK_MAX_FILES,
            max_bytes: DEFAULT_QUICK_MAX_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTierInput {
    pub changed_paths: Vec<String>,
    pub changed_bytes: u64,
    pub missing_path_count: usize,
    pub binary_paths: Vec<String>,
    pub special_paths: Vec<String>,
    pub is_worktree: bool,
    pub known_base: bool,
    pub current_line: String,
    pub default_line: String,
    pub workflow_mode: String,
    pub policy_profile: String,
    pub quick_limits: WorkflowTierLimits,
    pub extra_forbidden_prefixes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTierReason {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTierHighRiskPath {
    pub path: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTierCeremony {
    pub tier: WorkflowTier,
    pub minimum_commands: usize,
    pub records_created: usize,
    pub human_decisions: usize,
    pub recovery_steps: usize,
    pub wall_time_class: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTierEvaluation {
    pub contract: String,
    pub recommended_tier: WorkflowTier,
    pub quick_allowed: bool,
    pub reasons: Vec<WorkflowTierReason>,
    pub high_risk_paths: Vec<WorkflowTierHighRiskPath>,
    pub required_gates: Vec<String>,
    pub escalation_command: String,
    pub limits: WorkflowTierLimits,
    pub changed_path_count: usize,
    pub changed_bytes: u64,
    pub ceremony: Vec<WorkflowTierCeremony>,
}

pub fn workflow_tier_ceremony() -> Vec<WorkflowTierCeremony> {
    vec![
        WorkflowTierCeremony {
            tier: WorkflowTier::QuickModification,
            minimum_commands: 1,
            records_created: 1,
            human_decisions: 1,
            recovery_steps: 1,
            wall_time_class: "local_fast_validation".to_string(),
        },
        WorkflowTierCeremony {
            tier: WorkflowTier::NormalTask,
            minimum_commands: 3,
            records_created: 3,
            human_decisions: 2,
            recovery_steps: 2,
            wall_time_class: "focused_task_validation".to_string(),
        },
        WorkflowTierCeremony {
            tier: WorkflowTier::FullyGoverned,
            minimum_commands: 4,
            records_created: 8,
            human_decisions: 4,
            recovery_steps: 4,
            wall_time_class: "remote_ci_policy_review".to_string(),
        },
    ]
}

pub fn evaluate_workflow_tier(mut input: WorkflowTierInput) -> WorkflowTierEvaluation {
    input.changed_paths = normalized_sorted(input.changed_paths);
    input.binary_paths = normalized_sorted(input.binary_paths);
    input.special_paths = normalized_sorted(input.special_paths);
    input.extra_forbidden_prefixes = normalized_sorted(input.extra_forbidden_prefixes);
    if input.quick_limits.max_files == 0 {
        input.quick_limits.max_files = DEFAULT_QUICK_MAX_FILES;
    }
    if input.quick_limits.max_bytes == 0 {
        input.quick_limits.max_bytes = DEFAULT_QUICK_MAX_BYTES;
    }

    let binary_paths = input.binary_paths.iter().cloned().collect::<BTreeSet<_>>();
    let special_paths = input.special_paths.iter().cloned().collect::<BTreeSet<_>>();
    let mut high_risk_paths = Vec::new();
    for path in &input.changed_paths {
        let mut categories = classify_high_risk_path(path);
        if binary_paths.contains(path) {
            categories.insert("binary".to_string());
        }
        if special_paths.contains(path) {
            categories.insert("special_file".to_string());
        }
        if input
            .extra_forbidden_prefixes
            .iter()
            .any(|prefix| path_matches_prefix(path, prefix))
        {
            categories.insert("repository_rule".to_string());
        }
        if !categories.is_empty() {
            high_risk_paths.push(WorkflowTierHighRiskPath {
                path: path.clone(),
                categories: categories.into_iter().collect(),
            });
        }
    }

    let mut reasons = Vec::new();
    let mut governed = false;
    if !high_risk_paths.is_empty() {
        governed = true;
        reasons.push(reason(
            "protected_content",
            "Protected, planning, dependency, schema, migration, release, generated, binary, or repository-governed paths require the fully governed workflow.",
        ));
    }
    if input
        .workflow_mode
        .trim()
        .eq_ignore_ascii_case("team_remote")
    {
        governed = true;
        reasons.push(reason(
            "team_remote_mode",
            "team_remote mode keeps changes behind shared CI, policy, and review gates.",
        ));
    }
    if input.policy_profile.trim().eq_ignore_ascii_case("release") {
        governed = true;
        reasons.push(reason(
            "release_policy_profile",
            "The release policy profile cannot be downgraded to a quick or normal path.",
        ));
    }

    let mut quick_blocked = false;
    if input.changed_paths.is_empty() {
        quick_blocked = true;
        reasons.push(reason(
            "clean_workspace",
            "No workspace change exists to record.",
        ));
    }
    if input.is_worktree {
        quick_blocked = true;
        reasons.push(reason(
            "worktree_scope",
            "A task or other worktree must continue through its existing Task/Change lineage.",
        ));
    }
    if !input.known_base {
        quick_blocked = true;
        reasons.push(reason(
            "unknown_base",
            "Quick modification requires an existing immutable Snapshot as its known base.",
        ));
    }
    if input.current_line.trim().is_empty()
        || input.current_line.trim() == input.default_line.trim()
    {
        quick_blocked = true;
        reasons.push(reason(
            "protected_default_line",
            "Quick modification requires a non-default local line.",
        ));
    }
    if input.changed_paths.len() > input.quick_limits.max_files {
        quick_blocked = true;
        reasons.push(reason(
            "file_limit_exceeded",
            format!(
                "The workspace changes {} files; the configured quick limit is {}.",
                input.changed_paths.len(),
                input.quick_limits.max_files
            ),
        ));
    }
    if input.changed_bytes > input.quick_limits.max_bytes {
        quick_blocked = true;
        reasons.push(reason(
            "byte_limit_exceeded",
            format!(
                "The workspace contains {} changed bytes; the configured quick limit is {}.",
                input.changed_bytes, input.quick_limits.max_bytes
            ),
        ));
    }
    if input.missing_path_count > 0 {
        quick_blocked = true;
        reasons.push(reason(
            "deletion_requires_task",
            "Deleted content uses at least a normal Task so the removed bytes and recovery path remain reviewable.",
        ));
    }

    let recommended_tier = if governed {
        WorkflowTier::FullyGoverned
    } else if quick_blocked {
        WorkflowTier::NormalTask
    } else {
        reasons.push(reason(
            "bounded_low_risk_workspace",
            "The change is local, bounded, based on an immutable Snapshot, and contains no protected content.",
        ));
        WorkflowTier::QuickModification
    };
    let quick_allowed = recommended_tier == WorkflowTier::QuickModification;
    let (required_gates, escalation_command) = tier_route(recommended_tier);

    WorkflowTierEvaluation {
        contract: WORKFLOW_TIER_CONTRACT.to_string(),
        recommended_tier,
        quick_allowed,
        reasons,
        high_risk_paths,
        required_gates,
        escalation_command,
        limits: input.quick_limits,
        changed_path_count: input.changed_paths.len(),
        changed_bytes: input.changed_bytes,
        ceremony: workflow_tier_ceremony(),
    }
}

fn tier_route(tier: WorkflowTier) -> (Vec<String>, String) {
    match tier {
        WorkflowTier::QuickModification => (
            vec![
                "known_base".to_string(),
                "bounded_workspace".to_string(),
                "fast_validation_evidence".to_string(),
                "immutable_snapshot".to_string(),
            ],
            "ait snapshot create --profile quick --intent \"<intent>\" --validation \"<evidence>\" --message \"<message>\"".to_string(),
        ),
        WorkflowTier::NormalTask => (
            vec![
                "scoped_plan_binding".to_string(),
                "task_change_worktree".to_string(),
                "focused_validation".to_string(),
                "scope_appropriate_land".to_string(),
            ],
            "ait task start --from <sprint-card>#<exact-ref> --intent \"<intent>\" --base-line <line>".to_string(),
        ),
        WorkflowTier::FullyGoverned => (
            vec![
                "scoped_plan_binding".to_string(),
                "task_change_worktree".to_string(),
                "patchset".to_string(),
                "ci".to_string(),
                "attestation".to_string(),
                "policy".to_string(),
                "review".to_string(),
                "ready_land".to_string(),
            ],
            "ait task start --from <sprint-card>#<exact-ref> --intent \"<intent>\" --base-line <line>; then ait workflow ready <change-id> --apply; then ait task land <task-or-change-id>".to_string(),
        ),
    }
}

fn classify_high_risk_path(path: &str) -> BTreeSet<String> {
    let normalized = normalize_path(path);
    let components = normalized.split('/').collect::<Vec<_>>();
    let basename = components.last().copied().unwrap_or("");
    let mut categories = BTreeSet::new();

    if normalized == "agents.md"
        || normalized.starts_with(".ait/")
        || contains_component(&components, &["policy", "policies"])
    {
        categories.insert("policy".to_string());
    }
    if normalized == "docs/plan.md" || normalized.starts_with("docs/sprints/") {
        categories.insert("planning".to_string());
    }
    if contains_component(
        &components,
        &[
            "auth",
            "authentication",
            "authorization",
            "security",
            "secrets",
        ],
    ) {
        categories.insert("auth_security".to_string());
    }
    if contains_component(
        &components,
        &["schema", "schemas", "migration", "migrations"],
    ) || basename.ends_with(".sql")
    {
        categories.insert("schema_migration".to_string());
    }
    if normalized.starts_with("release/")
        || normalized.starts_with("packaging/")
        || normalized.starts_with(".github/workflows/")
        || matches!(basename, "dockerfile" | "containerfile")
    {
        categories.insert("release".to_string());
    }
    if is_dependency_manifest(basename) {
        categories.insert("dependency".to_string());
    }
    if contains_component(&components, &["generated", "vendor", "dist", "target"])
        || is_binary_extension(basename)
    {
        categories.insert("binary_generated".to_string());
    }
    categories
}

fn normalize_path(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn normalized_sorted(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().trim_start_matches("./").replace('\\', "/"))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn contains_component(components: &[&str], candidates: &[&str]) -> bool {
    components
        .iter()
        .any(|component| candidates.iter().any(|candidate| component == candidate))
}

fn is_dependency_manifest(basename: &str) -> bool {
    matches!(
        basename,
        "cargo.toml"
            | "cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "pyproject.toml"
            | "poetry.lock"
            | "pipfile"
            | "pipfile.lock"
            | "requirements.txt"
            | "go.mod"
            | "go.sum"
            | "gemfile"
            | "gemfile.lock"
            | "brewfile"
    ) || basename.starts_with("requirements-") && basename.ends_with(".txt")
}

fn is_binary_extension(basename: &str) -> bool {
    [
        ".a", ".bin", ".class", ".dll", ".dylib", ".exe", ".gif", ".ico", ".jar", ".jpeg", ".jpg",
        ".o", ".pdf", ".png", ".so", ".wasm", ".webp", ".zip",
    ]
    .iter()
    .any(|extension| basename.ends_with(extension))
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    let path = normalize_path(path);
    let prefix = normalize_path(prefix).trim_end_matches('/').to_string();
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn reason(code: &str, detail: impl Into<String>) -> WorkflowTierReason {
    WorkflowTierReason {
        code: code.to_string(),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(paths: &[&str]) -> WorkflowTierInput {
        WorkflowTierInput {
            changed_paths: paths.iter().map(|path| (*path).to_string()).collect(),
            changed_bytes: 128,
            missing_path_count: 0,
            binary_paths: Vec::new(),
            special_paths: Vec::new(),
            is_worktree: false,
            known_base: true,
            current_line: "quick/readme".to_string(),
            default_line: "main".to_string(),
            workflow_mode: "solo_remote".to_string(),
            policy_profile: "prototype".to_string(),
            quick_limits: WorkflowTierLimits::default(),
            extra_forbidden_prefixes: Vec::new(),
        }
    }

    #[test]
    fn workflow_tier_allows_a_bounded_low_risk_local_change() {
        let evaluation = evaluate_workflow_tier(input(&["README.md", "docs/usage.txt"]));
        assert_eq!(evaluation.recommended_tier, WorkflowTier::QuickModification);
        assert!(evaluation.quick_allowed);
        assert!(evaluation.high_risk_paths.is_empty());
    }

    #[test]
    fn workflow_tier_escalates_protected_and_dependency_paths() {
        for path in [
            ".ait/policy.yaml",
            "docs/plan.md",
            "docs/sprints/next-change.md",
            "src/auth/token.rs",
            "migrations/001.sql",
            ".github/workflows/release.yml",
            "Cargo.lock",
            "dist/ait.exe",
        ] {
            let evaluation = evaluate_workflow_tier(input(&[path]));
            assert_eq!(
                evaluation.recommended_tier,
                WorkflowTier::FullyGoverned,
                "{path}"
            );
            assert!(!evaluation.quick_allowed, "{path}");
            assert!(!evaluation.high_risk_paths.is_empty(), "{path}");
        }
    }

    #[test]
    fn workflow_tier_uses_normal_task_for_scope_size_and_deletion_escalation() {
        let mut value = input(&["src/lib.rs"]);
        value.current_line = "main".to_string();
        assert_eq!(
            evaluate_workflow_tier(value).recommended_tier,
            WorkflowTier::NormalTask
        );

        let mut value = input(&["src/lib.rs"]);
        value.changed_bytes = DEFAULT_QUICK_MAX_BYTES + 1;
        assert_eq!(
            evaluate_workflow_tier(value).recommended_tier,
            WorkflowTier::NormalTask
        );

        let mut value = input(&["src/lib.rs"]);
        value.missing_path_count = 1;
        assert_eq!(
            evaluate_workflow_tier(value).recommended_tier,
            WorkflowTier::NormalTask
        );
    }

    #[test]
    fn workflow_tier_never_downgrades_team_or_release_authority() {
        let mut team = input(&["README.md"]);
        team.workflow_mode = "team_remote".to_string();
        assert_eq!(
            evaluate_workflow_tier(team).recommended_tier,
            WorkflowTier::FullyGoverned
        );

        let mut release = input(&["README.md"]);
        release.policy_profile = "release".to_string();
        assert_eq!(
            evaluate_workflow_tier(release).recommended_tier,
            WorkflowTier::FullyGoverned
        );
    }

    #[test]
    fn workflow_tier_ceremony_is_strictly_monotonic() {
        let rows = workflow_tier_ceremony();
        assert_eq!(rows.len(), 3);
        for pair in rows.windows(2) {
            assert!(pair[0].minimum_commands < pair[1].minimum_commands);
            assert!(pair[0].records_created < pair[1].records_created);
            assert!(pair[0].human_decisions < pair[1].human_decisions);
            assert!(pair[0].recovery_steps < pair[1].recovery_steps);
        }
    }
}
