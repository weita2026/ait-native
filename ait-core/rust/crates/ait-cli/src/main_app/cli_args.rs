#[derive(Parser)]
#[command(name = "ait", version)]
#[command(about = "AIT native local repository and workflow tool.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Create or reinitialize a local AIT repository.")]
    Init(InitArgs),
    #[command(hide = true)]
    BinaryDb {
        #[command(subcommand)]
        command: BinaryDbCommand,
    },
    #[command(
        visible_alias = "branch",
        about = "Manage named Lines and their Snapshot heads.",
        long_about = "Manage named Lines and their Snapshot heads. `ait branch` is a Git-friendly alias with the same subcommands, safety checks, and output; the recorded objects remain AIT Lines and Snapshots."
    )]
    Line {
        #[command(subcommand)]
        command: LineCommand,
    },
    #[command(about = "Import, export, or mirror history between Git and AIT.")]
    Git {
        #[command(subcommand)]
        command: GitCommand,
    },
    #[command(about = "Attribute selected lines without modifying workspace files.")]
    Blame(BlameArgs),
    #[command(about = "Inspect effective AIT configuration and host readiness without repairing it.")]
    Doctor {
        #[command(subcommand)]
        command: DoctorCommand,
    },
    #[command(about = "Summarize active Tasks, Changes, worktrees, and reviews.")]
    Queue {
        #[command(subcommand)]
        command: QueueCommand,
    },
    #[command(about = "Manage server endpoints and recover the canonical remote main head.")]
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
    #[command(hide = true)]
    CurrentSourceCache {
        #[command(subcommand)]
        command: CurrentSourceCacheCommand,
    },
    #[command(about = "Build, inspect, and publish versioned release artifacts.")]
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    #[command(about = "Inspect and manage the configured Repository on its remote server.")]
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    #[command(hide = true)]
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    #[command(about = "Inspect effective configuration and change supported user settings.")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(
        about = "Manage reproducible external Repository dependencies.",
        long_about = "Inspect, diagnose, resolve, pin, restore, and locally link external Repository dependencies declared by ait-external.toml. Resolution writes an exact ait-external.lock; generated content is placed at each repository-relative destination declared in the manifest."
    )]
    External {
        #[command(subcommand)]
        command: ExternalCommand,
    },
    #[command(
        about = "Inspect the current repository, Line, Snapshot, workspace, and actionable local status.",
        long_about = "Inspect the current repository, Line, Snapshot, workspace, bounded maintenance findings, and actionable repair state without changing repository data or runtime state. Use --json for compact versioned machine-readable output; add --full for the complete output."
    )]
    Status(StatusArgs),
    #[command(
        about = "Compare the current workspace with the current Line head.",
        long_about = "Compare the current workspace with the current Line head without changing repository data or workspace files. With no output option, emit a unified text diff. Use --json for stable machine-readable output, --stat for per-file text statistics, or --name-only for the ordered changed-path list. Positional PATH values select specific workspace-relative files or directory subtrees using lexical matching, not globs."
    )]
    Diff(DiffArgs),
    #[command(
        about = "Import one remote Line and safely update its local head.",
        long_about = "Import the selected remote Line's reachable Snapshot chain and safely create or fast-forward its local Line. The configured default remote and current local Line are used when omitted. Workspace files remain unchanged unless --restore is supplied."
    )]
    Pull(PullArgs),
    #[command(
        about = "Upload one local Line and its Snapshot history to a remote.",
        long_about = "Upload an already-recorded local Line head and its required Snapshot history, then update the remote Line. Push does not capture workspace changes, create a Snapshot, publish a Patchset, or finish a Task."
    )]
    Push(PushArgs),
    #[command(
        about = "Inspect and safely maintain local content storage.",
        long_about = "Inspect bounded local content statistics, run exact read-only validation, or preview safe orphan object-pack cleanup. Pruning is read-only unless gc prune is given --apply; it never removes tree packs, individual unreachable blobs, Snapshots, Lines, Tasks, workspace files, or remote content."
    )]
    Gc {
        #[command(subcommand)]
        command: GcCommand,
    },
    #[command(
        about = "Park and restore temporary local-only workspace Snapshots.",
        long_about = "Park, inspect, restore, and drop temporary local-only workspace Snapshots without advancing Line heads. A stash can be restored only while its source Line is current."
    )]
    Stash {
        #[command(subcommand)]
        command: StashCommand,
    },
    #[command(
        about = "Inspect and sync local or remote Plans.",
        long_about = "Inspect, select, and sync Plan revision history. Without --local or --remote, solo_local uses local Plans and solo_remote uses the configured default remote. Either flag explicitly overrides that default. Remote sync updates local Plan history before publishing; no Plan command creates a Snapshot or advances a Line."
    )]
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    #[command(about = "Start, inspect, check, finish, or abandon Tasks locally or on a remote.")]
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    #[command(about = "Create, inspect, publish, or close Task-owned Changes.")]
    Change {
        #[command(subcommand)]
        command: ChangeCommand,
    },
    #[command(
        about = "Create and inspect immutable local Snapshots and restore selected workspace content.",
        long_about = "Create, inspect, compare, and query immutable local Snapshots. Snapshot create advances the current Line head; restore-lines, revert, and replay change only workspace files and never create a Snapshot, move a Line head, or change remote data."
    )]
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    #[command(
        about = "Create an AIT Snapshot using Git-friendly commit naming.",
        long_about = "Git-friendly alias for `ait snapshot create`. Capture the managed workspace as a new immutable local Snapshot and advance the current Line head. The result remains an AIT Snapshot, uses the same output and safety checks, and does not publish remote data."
    )]
    Commit(SnapshotCreateArgs),
    #[command(
        about = "Create, inspect, and delete local Snapshot Tags.",
        long_about = "Create, inspect, list, and delete local-only AIT Tags that name exact Snapshots. An existing Tag binding cannot be retargeted, and deleting a Tag never deletes its Snapshot."
    )]
    Tag {
        #[command(subcommand)]
        command: TagCommand,
    },
    #[command(
        about = "Publish and inspect remote Change revisions and their CI state.",
        long_about = "Publish the current local Line head as a remote Change revision, inspect or select specific published Patchsets, and read or manually rerun their remote CI. Published Patchsets exist only on remotes; --remote selects a configured remote and there is no local Patchset mode."
    )]
    Patchset {
        #[command(subcommand)]
        command: PatchsetCommand,
    },
    #[command(about = "Inspect remote Review state and record team, human Task, or AI code-review evidence.")]
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
    #[command(about = "Record and inspect evidence for remote Patchsets.")]
    Attest {
        #[command(subcommand)]
        command: AttestCommand,
    },
    #[command(about = "Check, inspect, or waive remote Patchset requirements.")]
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
    #[command(
        about = "Inspect, restore, recover, synchronize, rebase, and remove isolated worktrees.",
        long_about = "Inspect and maintain isolated worktrees that share repository .ait data while carrying their own checked-out files, current Line, and optional Task/Change binding. Normal Task worktrees are created by task start. Cleanup, prune-stale, and remove require --yes when applied; use --dry-run to preview destructive removal."
    )]
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
    #[command(about = "Prepare Changes for review, finish ready work, and repair workflow state.")]
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
}

#[derive(Subcommand)]
enum BinaryDbCommand {
    UpgradeU64Seconds(BinaryDbUpgradeU64SecondsArgs),
}

#[derive(Args, Clone)]
struct BinaryDbUpgradeU64SecondsArgs {
    #[arg(long = "repo-root", default_value = ".")]
    repo_root: PathBuf,
    #[arg(long = "generation-root")]
    generation_root: PathBuf,
    #[arg(long = "source-time-width")]
    source_time_width: String,
    #[arg(long)]
    activate: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct InitArgs {
    #[arg(
        long = "policy-profile",
        default_value = "prototype",
        help = "Initial policy profile: prototype, team, or release."
    )]
    policy_profile: String,
    #[arg(
        long = "repair-existing",
        help = "Complete missing structure in an existing .ait directory; invalid existing data is never overwritten."
    )]
    repair_existing: bool,
    #[arg(long, help = "Emit the stable JSON result.")]
    json: bool,
}

#[derive(Subcommand)]
enum GcCommand {
    #[command(
        about = "Show a bounded local content-storage summary.",
        long_about = "Show bounded local blob, tree, and pack counts and storage metrics without exact retained-tree traversal or full inventory rows. Use gc validate for exact retained-Snapshot reachability."
    )]
    Stats(GcStatsArgs),
    #[command(
        about = "Run exact read-only local content validation.",
        long_about = "Run exact read-only validation by traversing retained Snapshot trees and blobs and checking local pack metadata without modifying repository or workspace state. The result is emitted before the command returns nonzero when needs_attention is true."
    )]
    Validate(GcValidateArgs),
    #[command(
        about = "Preview safe orphan object-pack cleanup; mutate only with --apply.",
        long_about = "Preview the exact fully orphaned object-pack cleanup plan by default. --apply recomputes, verifies, and revalidates that plan before changing local catalog metadata and deleting unreferenced object-pack archives. This command never prunes tree packs, individual unreachable blobs, Snapshots, Lines, Tasks, workspace files, or remote content."
    )]
    Prune(GcPruneArgs),
}

#[derive(Args, Clone)]
struct GcStatsArgs {
    #[arg(long, help = "Emit the bounded summary as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct GcValidateArgs {
    #[arg(long, help = "Emit the exact validation result as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct GcPruneArgs {
    #[arg(
        long,
        help = "Apply the recomputed and revalidated safe plan; without this option the command is read-only."
    )]
    apply: bool,
    #[arg(long, help = "Emit the preview or apply result as stable JSON.")]
    json: bool,
}

#[derive(Subcommand)]
enum LineCommand {
    #[command(about = "List active local Lines or Lines from a remote.")]
    List(LineListArgs),
    #[command(about = "Create a new local Line at a Snapshot head.")]
    Create(LineCreateArgs),
    #[command(about = "Select a local Line, optionally restoring its files.")]
    Switch(LineSwitchArgs),
    #[command(about = "Show one local or remote Line and its current head.")]
    Show(LineShowArgs),
    #[command(about = "Archive a local or remote Line without deleting its Snapshots.")]
    Archive(LineArchiveArgs),
    #[command(about = "Rename a line while preserving its stable identity and reconciling bound pointers.")]
    Rename(LineRenameArgs),
    #[command(about = "Delete only a line ref after binding and unique-history protection checks.")]
    Delete(LineDeleteArgs),
    #[command(about = "Merge one Line into the current Line with resumable conflict state and a two-parent Snapshot.")]
    Merge(LineMergeArgs),
    #[command(about = "Preview idle temporary Lines, or archive eligible candidates with --yes.")]
    Cleanup(LineCleanupArgs),
}

#[derive(Subcommand)]
enum GitCommand {
    #[command(about = "Import Git commits, branches, and tags into AIT with a resumable immutable identity map.")]
    Import(GitImportArgs),
    #[command(about = "Export AIT lines, tags, and ordered Snapshot DAG history to Git without force-updating refs.")]
    Export(GitExportArgs),
    #[command(about = "Reconcile a Git endpoint and AIT through checkpointed, divergence-safe ref-set transactions.")]
    Mirror(GitMirrorArgs),
}

#[derive(Args, Clone)]
struct GitImportArgs {
    #[arg(
        value_name = "SOURCE",
        help = "Git source to import; accepts a local repository path or a Git remote URL."
    )]
    source: String,
    #[arg(
        long = "all-branches-and-tags",
        help = "Import every Git branch and tag instead of only the source HEAD branch."
    )]
    all_branches_and_tags: bool,
    #[arg(
        long,
        help = "Inspect and validate the immutable import plan without writing AIT data."
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct GitExportArgs {
    #[arg(
        value_name = "TARGET",
        help = "Local Git repository path to update, or a new path whose parent already exists."
    )]
    target: String,
    #[arg(
        long = "all-lines-and-tags",
        help = "Export every headed AIT Line and every AIT Tag instead of only the current Line."
    )]
    all_lines_and_tags: bool,
    #[arg(
        long,
        help = "Inspect and validate the immutable export plan without writing the target or AIT data."
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct GitMirrorArgs {
    #[arg(
        value_name = "ENDPOINT",
        help = "Git source for inbound mode; local Git path for outbound or bidirectional mode."
    )]
    endpoint: String,
    #[arg(
        long,
        value_name = "DIRECTION",
        value_parser = ["inbound", "outbound", "bidirectional"],
        help = "Select the permitted change direction; one-way modes block opposite-side changes and all modes block divergence."
    )]
    direction: String,
    #[arg(
        long,
        help = "Classify and validate the complete branch and tag set without writing changes."
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Subcommand)]
enum QueueCommand {
    #[command(about = "Show active local work and the configured remote review inbox.")]
    Summary(QueueSummaryArgs),
}

#[derive(Subcommand)]
enum RemoteCommand {
    #[command(about = "Register a remote using the canonical Repository directory name.")]
    Add(RemoteAddArgs),
    #[command(about = "List configured remotes without contacting their servers.")]
    List(RemoteListArgs),
    #[command(
        name = "recover-head",
        about = "Preview or atomically rebuild the exact remote main head and its full Snapshot ancestry."
    )]
    RecoverHead(RemoteRecoverHeadArgs),
}

#[derive(Subcommand)]
enum ExternalCommand {
    #[command(
        about = "Resolve and restore declared external Repositories.",
        long_about = "Resolve and restore declared external Repositories. With no NAME, preserve the manifest's exact pins while updating the complete dependency lock. NAME requires exactly one of --to or --latest. --locked is a separate target-free mode that reads an existing drift-free lockfile without changing the manifest or lockfile."
    )]
    Update(ExternalUpdateArgs),
    #[command(
        about = "Inspect external pins, links, lock drift, and restored content.",
        long_about = "Read the manifest, lockfile, local links, destination paths, and generated content without repairing or changing them. The report explains whether content is present, missing, linked, dirty, or outdated; lock drift is reported separately."
    )]
    Status(ExternalStatusArgs),
    #[command(
        about = "Evaluate external dependency release readiness without repairing it.",
        long_about = "Check the manifest, lockfile, restored content, local links, destination bindings, licenses, and applicable current-source readiness. The report is always emitted. By default findings are diagnostic only; --fail-on-blocking returns exit code 2 when release_ready is false."
    )]
    Doctor(ExternalDoctorArgs),
    #[command(
        about = "Use another local checkout for one declared direct external.",
        long_about = "Record a local development override in ait-external.links.toml. NAME must match exactly one direct ait-external.toml declaration. PATH must be an existing directory outside the current Repository; regular updates preserve the override, while locked restores and release checks reject active links."
    )]
    Link(ExternalLinkArgs),
    #[command(
        about = "Remove one local external override and restore locked content.",
        long_about = "Remove NAME from ait-external.links.toml. When ait-external.lock exists, restore the matching direct external and its recursive subtree before committing removal of the override. A failed restore leaves the logical link active."
    )]
    Unlink(ExternalUnlinkArgs),
}

#[derive(Args, Clone)]
#[command(
    override_usage = "ait external update [--validate] [--no-recursive] [--json]\n       ait external update <NAME> (--to <SNAPSHOT> | --latest) [--validate] [--no-recursive] [--json]\n       ait external update --locked [--validate] [--no-recursive] [--json]",
    group(
        ArgGroup::new("target_selection")
            .multiple(false)
            .args(["snapshot", "latest"])
    )
)]
struct ExternalUpdateArgs {
    #[arg(
        requires = "target_selection",
        conflicts_with = "locked",
        help = "Unique direct external name; requires --to or --latest."
    )]
    name: Option<String>,
    #[arg(
        long = "to",
        requires = "name",
        conflicts_with_all = ["latest", "locked"],
        help = "Pin NAME to this exact immutable Snapshot before resolving and restoring its files."
    )]
    snapshot: Option<String>,
    #[arg(
        long,
        requires = "name",
        conflicts_with_all = ["snapshot", "locked"],
        help = "Resolve NAME's declared remote and line head, then persist that exact Snapshot pin."
    )]
    latest: bool,
    #[arg(
        long,
        conflicts_with_all = ["name", "snapshot", "latest"],
        help = "Restore external files from the existing drift-free ait-external.lock without resolving or changing pins; active local links are rejected."
    )]
    locked: bool,
    #[arg(
        long,
        help = "Prepare the selected files and run destination toolchain checks first; install them only when validation has no errors."
    )]
    validate: bool,
    #[arg(
        long = "no-recursive",
        help = "Restore direct external dependencies only; the resolved lockfile still records the complete dependency graph."
    )]
    no_recursive: bool,
    #[arg(long, help = "Emit the complete machine-readable update report.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ExternalStatusArgs {
    #[arg(long, help = "Emit the complete machine-readable status report.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ExternalDoctorArgs {
    #[arg(
        long,
        help = "Return exit code 2 after emitting the report when release-blocking findings exist."
    )]
    fail_on_blocking: bool,
    #[arg(long, help = "Emit the complete machine-readable readiness report.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ExternalLinkArgs {
    #[arg(help = "Unique direct external name declared by ait-external.toml.")]
    name: String,
    #[arg(help = "Existing local checkout directory; relative paths resolve from the Repository root.")]
    path: String,
    #[arg(long, help = "Emit the complete machine-readable link result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ExternalUnlinkArgs {
    #[arg(help = "Local external override name to remove.")]
    name: String,
    #[arg(long, help = "Emit the complete machine-readable unlink and restoration result.")]
    json: bool,
}

#[derive(Subcommand)]
enum CurrentSourceCacheCommand {
    #[command(name = "run-cli", hide = true)]
    RunCli(CurrentSourceRunCliArgs),
    Contract(CurrentSourceCacheContractArgs),
    #[command(name = "mark-building")]
    MarkBuilding(CurrentSourceCacheStateArgs),
    Activate(CurrentSourceCacheActivateArgs),
    #[command(name = "release-lease")]
    ReleaseLease(CurrentSourceCacheReleaseLeaseArgs),
    Prune(CurrentSourceCachePruneArgs),
    #[command(name = "seed-canonical")]
    SeedCanonical(CurrentSourceCacheSeedCanonicalArgs),
    #[command(name = "extension-fresh")]
    ExtensionFresh(CurrentSourceExtensionFreshArgs),
    #[command(name = "binary-fresh")]
    BinaryFresh(CurrentSourceBinaryFreshArgs),
    #[command(name = "core-fingerprint")]
    CoreFingerprint(CurrentSourceFingerprintArgs),
    #[command(name = "server-fingerprint")]
    ServerFingerprint(CurrentSourceFingerprintArgs),
    #[command(name = "core-mtime")]
    CoreMtime(CurrentSourceFingerprintArgs),
    #[command(name = "server-mtime")]
    ServerMtime(CurrentSourceFingerprintArgs),
}

#[derive(Subcommand)]
enum ReleaseCommand {
    #[command(hide = true)]
    Adapter {
        #[command(subcommand)]
        command: ReleaseAdapterCommand,
    },
    Candidate {
        #[command(subcommand)]
        command: ReleaseCandidateCommand,
    },
    Check(ReleaseCheckArgs),
    Build(ReleaseBuildArgs),
    #[command(name = "native-source")]
    NativeSource(ReleaseNativeSourceArgs),
    #[command(name = "native-bundle")]
    NativeBundle(ReleaseNativeBundleArgs),
    Package(ReleasePackageArgs),
    Formula(ReleaseFormulaArgs),
    Show(ReleaseShowArgs),
    Publish(ReleasePublishArgs),
    Promote(ReleasePromoteArgs),
}

#[derive(Subcommand)]
enum ReleaseAdapterCommand {
    Check(ReleaseAdapterArgs),
    Build(ReleaseAdapterArgs),
}

#[derive(Subcommand)]
enum ReleaseCandidateCommand {
    Create(ReleaseCandidateCreateArgs),
}

#[derive(Subcommand)]
enum RepoCommand {
    #[command(about = "Read Repository identity, lifecycle, storage validation, and sync state.")]
    Show(RemoteJsonArgs),
    #[command(
        about = "Drain, archive, verify, and purge a remote Repository, or abort that retirement.",
        long_about = "Without --abort, stop new work for the remote Repository, download and verify its complete archive, then delete its server data. An unrelated complete local archive blocks retirement and must be handled with `ait repo restore --remote <NAME>`; there is no archive replacement option. With --abort, reactivate the Repository and preserve any complete local archive."
    )]
    Retire(RepoRetireArgs),
    #[command(
        about = "Restore a complete local retirement archive as a new remote Repository.",
        long_about = "Verify the complete `.ait/remote/<remote>/` retirement archive, create a new remote Repository index, upload and save its data, then update the locally configured Repository index. The archive determines the restored identity; there is no name, index, or force override."
    )]
    Restore(RemoteJsonArgs),
    #[command(about = "Read one Worker Job or a bounded, optionally filtered Job inventory.")]
    Jobs(RepoJobsArgs),
    #[command(
        name = "ci-capabilities",
        about = "Inspect server, native runner, and remote-sync prerequisites for Patchset CI."
    )]
    CiCapabilities(RemoteJsonArgs),
}

#[derive(Subcommand)]
enum AuthCommand {
    Whoami(AuthWhoamiArgs),
    Grant(AuthGrantArgs),
    Bindings(AuthBindingsArgs),
}

#[derive(Subcommand)]
enum ConfigCommand {
    #[command(
        about = "Inspect effective repository configuration without modifying it.",
        long_about = "Read repository configuration plus the applicable worktree settings and report the effective values. Human output is compact; --json returns the complete machine-readable result."
    )]
    Show(ConfigShowArgs),
    #[command(
        about = "Set supported user-owned repository defaults.",
        long_about = "Set one or more supported user-owned defaults, then refresh effective configuration and generated workflow guidance. Internal Repository indexes, derived local/remote settings, and Plan bindings cannot be set here. Use `ait config unset <KEY>` to remove an optional override."
    )]
    Set(Box<ConfigSetArgs>),
    #[command(
        about = "Remove one supported user setting.",
        long_about = "Delete exactly one optional user setting from repository configuration, then refresh effective configuration and generated workflow guidance. Fallbacks are: default-author-mode -> ai_with_human_review; default-model -> unset; task-review -> automatic; task-worktree-alias-root -> .ait-worktree-links; task-worktree-main-seed-ram-max-bytes -> no configured budget; id-namespace-prefix -> empty; user-name and user-email -> unset while actor detection remains available. Internal Repository indexes, workflow-mode, sprint, derived local/remote settings, and Plan bindings cannot be unset."
    )]
    Unset(ConfigUnsetArgs),
}

#[derive(Subcommand)]
enum DoctorCommand {
    #[command(
        about = "Validate the configured memory-backed runtime root.",
        long_about = "Validate the existing memory-backed root recorded in task_worktree.memory_root, plus its typed capacity and derived Task runtime location. This diagnostic never mounts, provisions, repairs, or creates the root."
    )]
    MemoryRoot(DoctorMemoryRootArgs),
    #[command(
        about = "Inspect the effective server runtime-root placement.",
        long_about = "Inspect the server runtime root selected by AIT_RUNTIME_DATA (or legacy AIT_NATIVE_SERVER_DATA) and report whether Snapshot scans protect it."
    )]
    RuntimeRoot(DoctorRuntimeRootArgs),
    #[command(
        about = "Inspect the fixed Rust-native Plan storage contract.",
        long_about = "Inspect the fixed Rust-native Plan storage contract and required exports."
    )]
    PlanAuthority(DoctorPlanAuthorityArgs),
}

#[derive(Args, Clone)]
struct LineListArgs {
    #[arg(long = "all", conflicts_with = "archived")]
    include_all: bool,
    #[arg(long, conflicts_with = "include_all")]
    archived: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineCreateArgs {
    name: String,
    #[arg(long = "from-snapshot", help = "Start the new Line at this Snapshot instead of the current Line head.")]
    from_snapshot: Option<String>,
    #[arg(long, help = "Select the new Line without changing workspace files.")]
    switch: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineSwitchArgs {
    name: String,
    #[arg(long, help = "Restore the selected Line head's files into the workspace.")]
    restore: bool,
    #[arg(long, requires = "restore", help = "Allow --restore to overwrite conflicting workspace changes.")]
    force: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineShowArgs {
    name: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineArchiveArgs {
    name: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineRenameArgs {
    old: String,
    new: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineDeleteArgs {
    name: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, required = true, action = ArgAction::SetTrue)]
    yes: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineMergeArgs {
    #[arg(conflicts_with_all = ["continue_merge", "abort_merge"])]
    source: Option<String>,
    #[arg(long, conflicts_with = "abort_merge")]
    message: Option<String>,
    #[arg(long = "continue", conflicts_with_all = ["source", "abort_merge"])]
    continue_merge: bool,
    #[arg(long = "abort", conflicts_with_all = ["source", "message", "continue_merge"])]
    abort_merge: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineCleanupArgs {
    #[arg(
        long = "idle-for",
        default_value = "7d",
        help = "Minimum time since the Line's last activity, such as 7d, 12h, or 30m."
    )]
    idle_for: String,
    #[arg(
        long = "kind",
        help = "Restrict cleanup to review_base, review, or wip Lines."
    )]
    cleanup_kind: Option<String>,
    #[arg(long, help = "Select at most this positive number of oldest candidates.")]
    limit: Option<usize>,
    #[arg(
        long = "include-protected",
        help = "Include non-candidates and their protection reasons in the result."
    )]
    include_protected: bool,
    #[arg(
        long,
        help = "Show every selected row; protected rows still require --include-protected."
    )]
    all: bool,
    #[arg(long, help = "Archive the eligible candidates; omission is always a read-only preview.")]
    yes: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct QueueSummaryArgs {
    #[arg(long, help = "Read the review inbox from this configured remote instead of the default remote.")]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete machine-readable summary.")]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteAddArgs {
    #[arg(help = "Local name used to select this remote.")]
    name: String,
    #[arg(help = "Base URL of the AIT server.")]
    url: String,
    #[arg(long = "default", help = "Use this remote by default for both push and pull.")]
    default: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteListArgs {
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteRecoverHeadArgs {
    #[arg(long, value_name = "NAME", help = "Read from this remote; otherwise use the configured default remote.")]
    remote: Option<String>,
    #[arg(
        long,
        default_value_t = 8,
        value_parser = parse_remote_recovery_jobs,
        help = "Use this many parallel pack downloads (1 through 64)."
    )]
    jobs: usize,
    #[arg(long, help = "Activate the reconstructed generation; omission performs a read-only preview.")]
    apply: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

fn parse_remote_recovery_jobs(value: &str) -> Result<usize, String> {
    let jobs = value
        .parse::<usize>()
        .map_err(|_| "jobs must be an integer from 1 through 64".to_string())?;
    if !(1..=64).contains(&jobs) {
        return Err("jobs must be from 1 through 64".to_string());
    }
    Ok(jobs)
}

#[derive(Args, Clone)]
struct CurrentSourceCachePathArgs {
    #[arg(long = "namespace-root")]
    namespace_root: PathBuf,
    #[arg(long = "core-repo-root")]
    core_repo_root: PathBuf,
    #[arg(long = "ext-suffix")]
    ext_suffix: String,
    #[arg(long, default_value = "")]
    rustflags: String,
    #[arg(long = "worker-id", default_value = "shared")]
    worker_id: String,
    #[arg(long = "core-source-fingerprint")]
    core_source_fingerprint: Option<String>,
    #[arg(long = "server-source-fingerprint")]
    server_source_fingerprint: Option<String>,
}

#[derive(Args, Clone)]
struct CurrentSourceRunCliArgs {
    #[arg(long = "metadata-path")]
    metadata_path: PathBuf,
    #[arg(long = "core-repo-root")]
    core_repo_root: PathBuf,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

#[derive(Args, Clone)]
struct CurrentSourceCacheContractArgs {
    #[command(flatten)]
    path: CurrentSourceCachePathArgs,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceCacheStateArgs {
    #[command(flatten)]
    path: CurrentSourceCachePathArgs,
    #[arg(long = "source-mtime-ns")]
    source_mtime_ns: u64,
    #[arg(long = "extra-json", default_value = "{}")]
    extra_json: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceCacheActivateArgs {
    #[command(flatten)]
    path: CurrentSourceCachePathArgs,
    #[arg(long = "source-mtime-ns")]
    source_mtime_ns: u64,
    #[arg(long = "extra-json", default_value = "{}")]
    extra_json: String,
    #[arg(long = "register-lease")]
    register_lease: bool,
    #[arg(long = "owner-pid", requires = "register_lease")]
    owner_pid: Option<u32>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceCacheReleaseLeaseArgs {
    #[arg(long = "lease-path")]
    lease_path: PathBuf,
    #[arg(long = "namespace-root")]
    namespace_root: PathBuf,
    #[arg(long = "remove-unleased-ready")]
    remove_unleased_ready: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceCachePruneArgs {
    #[arg(long = "namespace-root")]
    namespace_root: PathBuf,
    #[arg(long)]
    now: Option<f64>,
    #[arg(long = "idle-ttl-seconds", default_value_t = CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS)]
    idle_ttl_seconds: u64,
    #[arg(long = "build-stale-seconds", default_value_t = CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS)]
    build_stale_seconds: u64,
    #[arg(long = "max-bytes", default_value_t = CURRENT_SOURCE_CACHE_MAX_BYTES)]
    max_bytes: u64,
    #[arg(long = "remove-unleased-ready")]
    remove_unleased_ready: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceCacheSeedCanonicalArgs {
    #[command(flatten)]
    path: CurrentSourceCachePathArgs,
    #[arg(long = "repo-root")]
    repo_root: PathBuf,
    #[arg(long = "canonical-repo-root")]
    canonical_repo_root: PathBuf,
    #[arg(long = "core-source-mtime-ns")]
    core_source_mtime_ns: u64,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceExtensionFreshArgs {
    #[arg(long = "metadata-path")]
    metadata_path: PathBuf,
    #[arg(long = "extension-path")]
    extension_path: PathBuf,
    #[arg(long = "source-mtime-ns")]
    source_mtime_ns: u64,
    #[arg(long = "source-fingerprint")]
    source_fingerprint: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceBinaryFreshArgs {
    #[arg(long = "metadata-path")]
    metadata_path: PathBuf,
    #[arg(long = "binary-path")]
    binary_path: PathBuf,
    #[arg(long = "metadata-fingerprint-key")]
    metadata_fingerprint_key: String,
    #[arg(long = "metadata-source-mtime-key")]
    metadata_source_mtime_key: String,
    #[arg(long = "metadata-mtime-key")]
    metadata_mtime_key: String,
    #[arg(long = "metadata-sha-key")]
    metadata_sha_key: String,
    #[arg(long = "source-mtime-ns")]
    source_mtime_ns: u64,
    #[arg(long = "source-fingerprint")]
    source_fingerprint: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct CurrentSourceFingerprintArgs {
    repo_root: PathBuf,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseCandidateCreateArgs {
    #[arg(long)]
    version: String,
    #[arg(long = "line", default_value = "main")]
    line_name: String,
    #[arg(long)]
    profile: Option<String>,
    #[arg(long, value_parser = ["rc", "stable"])]
    channel: Option<String>,
    #[arg(
        long = "public-source-root",
        value_name = "DIR",
        hide = true,
        help = "Protected CI only: reconstruct the exact family candidate from an exported public Git source mapping"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseAdapterArgs {
    #[arg(long)]
    version: String,
    #[arg(long = "line", default_value = "main")]
    line_name: String,
    #[arg(
        long,
        help = "Internal CI selector: build only artifacts declared for this target"
    )]
    target: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseCheckArgs {
    release_id: String,
    #[arg(
        long = "receipts",
        value_name = "DIR",
        help = "Verify family component receipts and artifact bytes below DIR"
    )]
    receipts: Option<PathBuf>,
    #[arg(
        long = "public-source-root",
        value_name = "DIR",
        hide = true,
        help = "Protected CI only: retain the exported public Git source while verifying family receipts"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(long = "tests-command")]
    tests_command: Option<String>,
    #[arg(long = "skip-tests-reason")]
    skip_tests_reason: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseBuildArgs {
    release_id: String,
    #[arg(
        long = "receipts",
        value_name = "DIR",
        help = "Freeze family artifacts verified from component receipts below DIR"
    )]
    receipts: Option<PathBuf>,
    #[arg(
        long = "public-source-root",
        value_name = "DIR",
        hide = true,
        help = "Protected CI only: retain the exported public Git source while freezing family artifacts"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(
        long = "native-matrix-dir",
        value_name = "DIR",
        help = "Read target artifacts from DIR/<rust-target-triple>/release using ait-native-source.json descriptors."
    )]
    native_matrix_dir: Option<PathBuf>,
    #[arg(
        long = "native-command-dir",
        value_name = "DIR",
        help = "Read current-host release command artifacts from this explicit directory."
    )]
    native_command_dir: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseNativeSourceArgs {
    release_id: String,
    #[arg(long)]
    target: String,
    #[arg(long = "source-dir", value_name = "DIR")]
    source_dir: PathBuf,
    #[arg(long)]
    runner: String,
    #[arg(long = "runner-image")]
    runner_image: String,
    #[arg(long = "rust-toolchain")]
    rust_toolchain: String,
    #[arg(long = "rustc-path", value_name = "PATH")]
    rustc_path: PathBuf,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseNativeBundleArgs {
    release_id: String,
    #[arg(
        long = "native-matrix-dir",
        value_name = "DIR",
        help = "Build native bundles only from DIR/<rust-target-triple>/release using ait-native-source.json descriptors."
    )]
    native_matrix_dir: PathBuf,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleasePackageArgs {
    release_id: String,
    #[arg(
        long,
        value_parser = ["homebrew", "apt", "winget", "pypi", "npm"],
        help = "Assemble one declared channel from an immutable frozen family build"
    )]
    channel: String,
    #[arg(
        long = "public-source-root",
        value_name = "DIR",
        hide = true,
        help = "Protected CI only: retain the exported public Git source while assembling a frozen family channel"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseFormulaArgs {
    release_id: String,
    #[arg(long)]
    name: String,
    #[arg(
        long = "python-formula",
        default_value = ait_cli::release_surface::DEFAULT_HOMEBREW_PYTHON_FORMULA,
        help = "Homebrew Python dependency formula written into the generated formula."
    )]
    python_formula: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleaseShowArgs {
    release_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(
        long = "public-source-root",
        value_name = "DIR",
        hide = true,
        help = "Protected CI only: retain the exported public Git source while inspecting a family dossier"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleasePublishArgs {
    release_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReleasePromoteArgs {
    release_id: String,
    #[arg(long, value_parser = ["rc", "stable"])]
    channel: String,
    #[arg(
        long = "public-source-root",
        value_name = "DIR",
        hide = true,
        help = "Protected CI only: retain the exported public Git source while emitting the family promotion handoff"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteJsonArgs {
    #[arg(
        long,
        value_name = "NAME",
        help = "Use this configured remote; omission uses the Repository default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct RepoRetireArgs {
    #[arg(
        long,
        value_name = "NAME",
        help = "Mutate this configured remote; omission uses the Repository default remote."
    )]
    remote: Option<String>,
    #[arg(
        long,
        help = "Abort an in-progress retirement and reactivate the Repository while preserving any complete local archive."
    )]
    abort: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct RepoJobsArgs {
    #[arg(
        long,
        value_name = "NAME",
        help = "Read from this configured remote; omission uses the Repository default remote."
    )]
    remote: Option<String>,
    #[arg(
        long = "worker-job-index",
        value_name = "INDEX",
        conflicts_with_all = ["state", "limit"],
        help = "Read one specific Worker Job from the current Repository; cannot be combined with list filters."
    )]
    worker_job_index: Option<u32>,
    #[arg(
        long,
        value_name = "STATE",
        value_parser = ["queued", "running", "succeeded", "failed"],
        conflicts_with = "worker_job_index",
        help = "Return only queued, running, succeeded, or failed Jobs."
    )]
    state: Option<String>,
    #[arg(
        long,
        value_name = "COUNT",
        default_value_t = ait_core::server_operational::WORKER_JOB_LIST_LIMIT_DEFAULT,
        value_parser = parse_repo_jobs_limit,
        conflicts_with = "worker_job_index",
        help = "Return at most this many Jobs in list mode (1 through 1000; default 50)."
    )]
    limit: u32,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

fn parse_repo_jobs_limit(value: &str) -> Result<u32, String> {
    let limit = value
        .parse::<u32>()
        .map_err(|_| "limit must be an integer from 1 through 1000".to_string())?;
    ait_core::server_operational::validate_worker_job_list_limit(limit).map_err(|_| {
        format!(
            "limit must be from {} through {}",
            ait_core::server_operational::WORKER_JOB_LIST_LIMIT_MIN,
            ait_core::server_operational::WORKER_JOB_LIST_LIMIT_MAX
        )
    })?;
    Ok(limit)
}

#[derive(Args, Clone)]
struct AuthWhoamiArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct AuthGrantArgs {
    #[arg(long)]
    actor: String,
    #[arg(long)]
    role: Vec<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct AuthBindingsArgs {
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct StatusArgs {
    #[arg(long, help = "Emit compact versioned machine-readable status.")]
    json: bool,
    #[arg(
        long,
        requires = "json",
        help = "With --json, emit the complete status result."
    )]
    full: bool,
}

#[derive(Args, Clone)]
struct DiffArgs {
    #[arg(
        long,
        conflicts_with_all = ["stat", "name_only"],
        help = "Emit the stable machine-readable workspace diff."
    )]
    json: bool,
    #[arg(
        long,
        conflicts_with_all = ["json", "name_only"],
        help = "Emit per-file text statistics."
    )]
    stat: bool,
    #[arg(
        long = "name-only",
        conflicts_with_all = ["json", "stat"],
        help = "Emit only the ordered changed-path list."
    )]
    name_only: bool,
    #[arg(
        value_name = "PATH",
        help = "Limit the comparison to an exact workspace-relative file or directory subtree.",
        long_help = "Limit the comparison to an exact workspace-relative file or directory subtree. Repeat PATH to combine lexical filters; glob syntax is not supported."
    )]
    paths: Vec<String>,
}

#[derive(Args, Clone)]
struct PullArgs {
    #[arg(
        long,
        help = "Use this configured remote; defaults to the repository's default remote."
    )]
    remote: Option<String>,
    #[arg(
        long,
        help = "Import this remote Line and safely create or fast-forward its local Line; defaults to the current local Line."
    )]
    line: Option<String>,
    #[arg(
        long,
        requires = "restore",
        help = "Merge a divergent imported remote head into the current local Line and restore the resulting files; requires --restore and a clean workspace."
    )]
    merge: bool,
    #[arg(
        long,
        help = "Restore the pulled Line's files into the workspace and select that Line; rejected when the local Line is ahead of the remote."
    )]
    restore: bool,
    #[arg(
        long,
        requires = "restore",
        conflicts_with = "merge",
        help = "Allow --restore to overwrite local workspace changes; requires --restore and cannot be used with --merge."
    )]
    force: bool,
    #[arg(long, help = "Emit the pull result as machine-readable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct PushArgs {
    #[arg(long, help = "Upload to this configured remote instead of the repository default.")]
    remote: Option<String>,
    #[arg(long, help = "Upload this local Line instead of the current Line.")]
    line: Option<String>,
    #[arg(long, help = "Emit the complete machine-readable push result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorMemoryRootArgs {
    #[arg(long, help = "Emit the diagnostic payload as JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorRuntimeRootArgs {
    #[arg(long, help = "Emit the diagnostic payload as JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorPlanAuthorityArgs {
    #[arg(long, help = "Emit the diagnostic payload as JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct BlameArgs {
    path: String,
    #[arg(
        long,
        value_parser = parse_positive_line,
        conflicts_with_all = ["start_line", "end_line"],
        help = "Return blame for one line only."
    )]
    line: Option<usize>,
    #[arg(
        long = "start",
        value_parser = parse_positive_line,
        requires = "end_line",
        conflicts_with = "line",
        help = "Start line for a bounded blame range."
    )]
    start_line: Option<usize>,
    #[arg(
        long = "end",
        value_parser = parse_positive_line,
        requires = "start_line",
        conflicts_with = "line",
        help = "End line for a bounded blame range."
    )]
    end_line: Option<usize>,
    #[arg(
        long = "snapshot",
        conflicts_with_all = ["patchset_id", "plan_id", "plan_ref"],
        help = "Blame against one explicit immutable snapshot."
    )]
    snapshot_id: Option<String>,
    #[arg(
        long = "via-parent",
        requires = "snapshot_id",
        help = "For an explicit merge Snapshot, follow this direct parent instead of the primary parent."
    )]
    via_parent_snapshot_id: Option<String>,
    #[arg(
        long = "patchset",
        value_parser = parse_exact_patchset_id,
        conflicts_with_all = ["snapshot_id", "plan_id", "plan_ref"],
        help = "Resolve one exact published Patchset ID to its revision Snapshot before blaming."
    )]
    patchset_id: Option<String>,
    #[arg(
        long = "remote",
        requires = "patchset_id",
        help = "Remote to use when resolving --patchset."
    )]
    remote_name: Option<String>,
    #[arg(
        long = "plan-id",
        conflicts_with_all = ["plan_ref", "snapshot_id", "patchset_id"],
        help = "Select one current Markdown Plan when the same file path is tracked by multiple current Plans."
    )]
    plan_id: Option<String>,
    #[arg(
        long = "plan-ref",
        conflicts_with_all = ["plan_id", "snapshot_id", "patchset_id"],
        help = "Select one current Markdown Plan by file selector or reference."
    )]
    plan_ref: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ConfigShowArgs {
    #[arg(long, help = "Emit the complete machine-readable effective configuration.")]
    json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ConfigWorkflowModeArg {
    #[value(name = "solo_local")]
    SoloLocal,
    #[value(name = "solo_remote")]
    SoloRemote,
    #[value(name = "team_remote")]
    TeamRemote,
}

impl ConfigWorkflowModeArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::SoloLocal => "solo_local",
            Self::SoloRemote => "solo_remote",
            Self::TeamRemote => "team_remote",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ConfigToggleArg {
    On,
    Off,
}

impl ConfigToggleArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ConfigAuthorModeArg {
    #[value(name = "human_only")]
    HumanOnly,
    #[value(name = "human_with_ai_assist")]
    HumanWithAiAssist,
    #[value(name = "ai_with_human_review")]
    AiWithHumanReview,
    #[value(name = "ai_only_experimental")]
    AiOnlyExperimental,
}

impl ConfigAuthorModeArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::HumanOnly => "human_only",
            Self::HumanWithAiAssist => "human_with_ai_assist",
            Self::AiWithHumanReview => "ai_with_human_review",
            Self::AiOnlyExperimental => "ai_only_experimental",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ConfigTaskReviewArg {
    Required,
    Automatic,
}

impl ConfigTaskReviewArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Automatic => "automatic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ConfigUnsetKeyArg {
    #[value(name = "default-author-mode")]
    DefaultAuthorMode,
    #[value(name = "default-model")]
    DefaultModel,
    #[value(name = "task-review")]
    TaskReview,
    #[value(name = "task-worktree-alias-root")]
    TaskWorktreeAliasRoot,
    #[value(name = "task-worktree-main-seed-ram-max-bytes")]
    TaskWorktreeMainSeedRamMaxBytes,
    #[value(name = "id-namespace-prefix")]
    IdNamespacePrefix,
    #[value(name = "user-name")]
    UserName,
    #[value(name = "user-email")]
    UserEmail,
}

impl ConfigUnsetKeyArg {
    fn into_config_key(self) -> ConfigUnsetKey {
        match self {
            Self::DefaultAuthorMode => ConfigUnsetKey::DefaultAuthorMode,
            Self::DefaultModel => ConfigUnsetKey::DefaultModel,
            Self::TaskReview => ConfigUnsetKey::TaskReview,
            Self::TaskWorktreeAliasRoot => ConfigUnsetKey::TaskWorktreeAliasRoot,
            Self::TaskWorktreeMainSeedRamMaxBytes => {
                ConfigUnsetKey::TaskWorktreeMainSeedRamMaxBytes
            }
            Self::IdNamespacePrefix => ConfigUnsetKey::IdNamespacePrefix,
            Self::UserName => ConfigUnsetKey::UserName,
            Self::UserEmail => ConfigUnsetKey::UserEmail,
        }
    }
}

#[derive(Args, Clone)]
struct ConfigSetArgs {
    #[arg(
        long = "workflow-mode",
        value_enum,
        help = "Set the complete workflow preset. This chooses where Workflow, Task, and Change commands run and defaults sprint mode to on unless --sprint is supplied."
    )]
    workflow_mode: Option<ConfigWorkflowModeArg>,
    #[arg(
        long = "sprint",
        value_enum,
        help = "Set sprint-style Plan/Task binding. On requires exact Plan item refs; off uses unbound Tasks."
    )]
    sprint: Option<ConfigToggleArg>,
    #[arg(
        long = "default-author-mode",
        value_enum,
        help = "Set the default provenance author mode."
    )]
    default_author_mode: Option<ConfigAuthorModeArg>,
    #[arg(long = "default-model", help = "Set a non-empty default provenance model name.")]
    default_model: Option<String>,
    #[arg(
        long = "task-review",
        value_enum,
        help = "Set Task outcome review policy. Required waits for review; automatic records task_approve using configured user_name."
    )]
    task_review: Option<ConfigTaskReviewArg>,
    #[arg(
        long = "task-worktree-alias-root",
        help = "Set a non-empty managed alias root for task worktrees. Relative paths resolve from the Repository root."
    )]
    task_worktree_alias_root: Option<String>,
    #[arg(
        long = "task-worktree-main-seed-ram-max-bytes",
        help = "Set a non-negative Repository-local RAM budget in bytes for main-seed-backed task worktree bootstrap."
    )]
    task_worktree_main_seed_ram_max_bytes: Option<i64>,
    #[arg(
        long = "id-namespace-prefix",
        help = "Set a non-empty ASCII-alphanumeric namespace prefix before workflow type codes such as T/C/P/PL/PR."
    )]
    id_namespace_prefix: Option<String>,
    #[arg(
        long = "user-name",
        help = "Set the non-empty human identity used by explicit and automatic Task review."
    )]
    user_name: Option<String>,
    #[arg(
        long = "user-email",
        help = "Set a non-empty local actor email for non-Task-review identity surfaces."
    )]
    user_email: Option<String>,
    #[arg(long, help = "Emit the complete machine-readable effective configuration after the update.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ConfigUnsetArgs {
    #[arg(
        value_enum,
        help = "Supported optional user setting to remove."
    )]
    key: ConfigUnsetKeyArg,
    #[arg(long, help = "Emit the complete machine-readable effective configuration after removal.")]
    json: bool,
}

#[derive(Subcommand)]
enum PlanCommand {
    #[command(
        about = "List local Plans or Plans from a remote.",
        long_about = "List Plans locally, from --remote, or from the location selected by the workflow mode. Human output is bounded to active Plans unless --all is supplied; JSON always returns the complete command result and therefore rejects --all. This command is read-only."
    )]
    List(PlanListArgs),
    #[command(
        about = "Show one Plan and its selected revision.",
        long_about = "Show one Plan locally, from --remote, or from the location selected by the workflow mode. Without --revision, show the current head revision. Local selectors include PR-<ordinal>, artifact:<path>, title:<title>, and published-plan:<index>; remote selectors are the identifiers returned by remote plan list. This command is read-only."
    )]
    Show(ShowArgs),
    #[command(
        about = "List revision history for one Plan.",
        long_about = "List one Plan's revisions newest first, using local data, --remote, or the location selected by the workflow mode. Human output is bounded unless --all is supplied; JSON already returns the complete command result and therefore rejects --all. This command is read-only."
    )]
    Revisions(PlanIdArgs),
    #[command(
        about = "List checklist items from one Plan revision.",
        long_about = "List parsed checklist items from one local or remote Plan. Without --revision, inspect the current head. This command is read-only and does not change item or Task state."
    )]
    Items(ShowArgs),
    #[command(
        about = "Find active Plans that can supply Task candidates.",
        long_about = "Find active local or remote Plans, ordered for Task selection. By default only Plans with at least one taskable item are returned; --all also includes active Plans with zero taskable items. --contains applies case-insensitive OR matching across Plan titles, file paths and selectors, headings, and item text or refs. This command is read-only."
    )]
    Candidates(CandidatesArgs),
    #[command(
        about = "Inspect Task-readiness details for one Plan revision.",
        long_about = "Inspect summary counts and per-item Task-readiness blockers for one local or remote Plan. Without --revision, inspect the current head. This command is read-only."
    )]
    Inspect(ShowArgs),
    #[command(
        about = PLAN_SYNC_COMMAND_ABOUT,
        long_about = PLAN_SYNC_COMMAND_ABOUT
    )]
    Sync(SyncArgs),
}

#[derive(Subcommand)]
enum TaskCommand {
    #[command(
        about = "Start one Task and its initial Change locally or on a remote; sprint mode uses one specific file-backed Plan item",
        override_usage = "ait task start --intent <INTENT> (--from <MARKDOWN_PATH#ITEM_REF> | --title <TITLE>) [--local | --remote <REMOTE>] [--json [--full]]"
    )]
    Start(TaskStartArgs),
    #[command(about = "List open Tasks or complete Task history locally or on a remote")]
    List(TaskListArgs),
    #[command(about = "Show one local or remote Task")]
    Show(TaskShowArgs),
    #[command(about = "Check one local or remote Task against main without changing it")]
    Audit(TaskAuditArgs),
    #[command(
        about = TASK_FINISH_COMMAND_ABOUT
    )]
    Finish(TaskFinishArgs),
    #[command(
        about = "Permanently abandon one local or remote Task and cancel its open Changes"
    )]
    Abandon(TaskAbandonArgs),
}

#[derive(Subcommand)]
enum ChangeCommand {
    #[command(about = "Create an additional Change for an existing Task.")]
    Create(ChangeCreateArgs),
    #[command(about = "List open Changes or complete Change history locally or on a remote.")]
    List(ChangeListArgs),
    #[command(about = "Inspect one Task-owned Change without changing it.")]
    Show(ChangeShowArgs),
    #[command(
        about = "Remove one Change's recorded delta from the current workspace.",
        long_about = "Remove one Change's recorded fork-to-revision delta from the current workspace. This does not create a Snapshot, move the current Line head, close the Change, or change remote data."
    )]
    Revert(ChangeRevertArgs),
    #[command(
        about = "Apply one Change's recorded delta to the current workspace.",
        long_about = "Apply one Change's recorded fork-to-revision delta to the current Line workspace. This does not create a Snapshot, move the Line head, land the Change, or change remote data."
    )]
    Replay(ChangeReplayArgs),
    #[command(
        about = "Archive one Change without landing it.",
        long_about = "Archive one local or remote Change without landing code. A successful close also attempts a bounded safe Plan repair for the owning Task."
    )]
    Close(ChangeCloseArgs),
    #[command(
        about = "Promote one local draft Change record to a remote.",
        long_about = "Promote one local draft Change record to the configured or named remote. This does not publish a Patchset, publish current workspace content, or land the Change."
    )]
    Publish(ChangePublishArgs),
}

#[derive(Subcommand)]
enum SnapshotCommand {
    #[command(
        about = "Capture the managed workspace as a new Snapshot.",
        long_about = "Capture the managed workspace as a new immutable local Snapshot and advance the current Line head. This command does not publish remote data."
    )]
    Create(SnapshotCreateArgs),
    #[command(
        about = "List repository-wide local Snapshot history.",
        long_about = "List local Snapshots across every Line without modifying repository state. Text and JSON output are bounded to the recent view by default; --all emits complete history."
    )]
    List(SnapshotListArgs),
    #[command(
        about = "Inspect one immutable local Snapshot.",
        long_about = "Inspect one Snapshot resolved from an exact Snapshot ID or local AIT Tag. Default text includes identity and bounded primary-parent change evidence; --files expands the complete tree inventory. JSON is always complete."
    )]
    Show(SnapshotShowArgs),
    #[command(
        about = "Compare two immutable local Snapshots.",
        long_about = "Compare two Snapshots resolved from exact Snapshot IDs or local AIT Tags. Structural file changes are reported by default; --include-text adds bounded text diffs for eligible modified files. This command is read-only."
    )]
    Diff(SnapshotDiffArgs),
    #[command(
        name = "restore-lines",
        about = "Preview or apply an exact line range from one Snapshot into the workspace.",
        long_about = "Read one exact immutable Snapshot ID and preview replacement of the same existing regular workspace path at one positive 1-based line or inclusive range. Only --yes applies the selected lines; the command never creates a Snapshot or moves a Line head."
    )]
    RestoreLines(SnapshotRestoreLinesArgs),
    #[command(
        about = "Remove the current head Snapshot's recorded delta from the workspace.",
        long_about = "Remove one Line Snapshot's primary-parent-to-revision delta from the current workspace. The requested Snapshot must be the current Line head. This does not create a Snapshot, move the current Line head, or mutate remote state."
    )]
    Revert(SnapshotRevertArgs),
    #[command(
        about = "Apply one Snapshot's recorded delta to the current Line workspace.",
        long_about = "Apply one non-root Line Snapshot's primary-parent-to-revision delta to the current Line workspace. This does not create a Snapshot, move the current Line head, or mutate remote state."
    )]
    Replay(SnapshotReplayArgs),
    #[command(
        about = "Query bounded Snapshot ancestors or descendants using metadata-only DAG traversal.",
        long_about = "Query deterministic topological Snapshot ancestry resolved from an exact Snapshot ID or local AIT Tag. Ancestors across all parents are the default; direction, first-parent traversal, depth, and result bounds are explicit. The query Snapshot itself is excluded."
    )]
    Ancestry(SnapshotAncestryArgs),
    #[command(
        name = "is-ancestor",
        about = "Test DAG ancestry; exits 0 when true, 1 when false, and 2 on lookup or storage errors.",
        long_about = "Test whether one Snapshot is reachable as an ancestor of another across the complete parent DAG. Each input accepts an exact Snapshot ID or local AIT Tag. The command exits 0 when true, 1 when false, and 2 on lookup or storage errors."
    )]
    IsAncestor(SnapshotIsAncestorArgs),
    #[command(
        name = "merge-base",
        about = "Find the deterministic best common Snapshot ancestor.",
        long_about = "Find the best common ancestor of two Snapshots resolved from exact Snapshot IDs or local AIT Tags. The default emits one deterministic best base; --all emits every equally best base. The command exits 0 when a base exists, 1 when none exists, and 2 on lookup or storage errors."
    )]
    MergeBase(SnapshotMergeBaseArgs),
}

#[derive(Subcommand)]
enum StashCommand {
    #[command(
        about = "Save modified workspace content as a temporary local-only stash.",
        long_about = "Save modified managed-workspace content as a temporary local-only stash Snapshot without advancing the current Line head. By default, restore the current Line head into the workspace after saving; --keep-workspace leaves the saved files in place."
    )]
    Save(StashSaveArgs),
    #[command(
        about = "List active local-only stash metadata without changing the workspace."
    )]
    List(StashListArgs),
    #[command(
        about = "Inspect metadata for one active local-only stash.",
        long_about = "Inspect metadata for one active local-only stash without changing the workspace. This command reports the stash record and Snapshot summary; it does not display a content diff."
    )]
    Show(StashIdArgs),
    #[command(
        about = "Restore a same-Line stash and retain its stash record.",
        long_about = "Replace the entire managed workspace with an active stash Snapshot and retain its stash record, without moving the current Line head. The current Line must be the stash's source Line. This restores the complete workspace rather than applying a patch or three-way merge."
    )]
    Apply(StashRestoreArgs),
    #[command(
        about = "Restore a same-Line stash and then drop its stash record.",
        long_about = "Replace the entire managed workspace with an active stash Snapshot and drop its stash record only after a successful restore, without moving the current Line head. The current Line must be the stash's source Line. This restores the complete workspace rather than applying a patch or three-way merge."
    )]
    Pop(StashRestoreArgs),
    #[command(
        about = "Drop an active stash record without changing workspace content."
    )]
    Drop(StashIdArgs),
}

#[derive(Subcommand)]
enum TagCommand {
    #[command(
        about = "Create a local Tag; existing names are rejected.",
        long_about = "Create one local AIT Tag for an exact existing Snapshot. When --snapshot is omitted, the current Line head is used. An existing Tag name is always rejected and cannot be replaced or moved."
    )]
    Create(TagCreateArgs),
    #[command(about = "List all local Tags without changing repository state.")]
    List(TagListArgs),
    #[command(about = "Show one exact local Tag without changing repository state.")]
    Show(TagShowArgs),
    #[command(
        about = "Delete only one local Tag reference.",
        long_about = "Delete one exact local AIT Tag reference. The referenced Snapshot, Line heads, and workspace content remain unchanged."
    )]
    Delete(TagDeleteArgs),
}

#[derive(Subcommand)]
enum PatchsetCommand {
    #[command(
        about = "Publish the current Line head as a new remote Patchset.",
        long_about = "Publish the current local Line head as the next Patchset for CHANGE_ID after validating the bound worktree and synchronizing its revision Snapshot to the selected remote. --summary is required; --author-mode overrides the configured provenance mode for this publication only."
    )]
    Publish(PatchsetPublishArgs),
    #[command(about = "List the published Patchsets owned by one remote Change without modifying it.")]
    List(PatchsetListArgs),
    #[command(about = "Show one exact published remote Patchset without modifying it.")]
    Show(PatchsetShowArgs),
    #[command(
        about = "Select one specific Patchset on its owning remote Change.",
        long_about = "Read the requested Patchset, find its owning remote Change, then make that Patchset the Change's selected revision. The owning Change cannot be supplied or overridden by the caller."
    )]
    Select(PatchsetSelectArgs),
    #[command(
        name = "ci-status",
        about = "Read CI state for one exact remote Patchset.",
        long_about = "Read current CI readiness and the 10 most recent CI jobs for one published Patchset without changing remote data."
    )]
    CiStatus(PatchsetCiStatusArgs),
    #[command(
        name = "rerun-ci",
        about = "Queue a manual CI rerun for one exact remote Patchset.",
        long_about = "Queue CI for one exact published Patchset using the fixed trigger manual_rerun. Runner selection and execution profiles remain server policy and cannot be overridden here."
    )]
    RerunCi(PatchsetRerunCiArgs),
}

#[derive(Subcommand)]
enum ReviewCommand {
    #[command(about = "Show compact remote Review state for one Change.")]
    Show(ReviewShowArgs),
    #[command(about = "Manage team governance review; available only in team_remote mode.")]
    Team {
        #[command(subcommand)]
        command: ReviewTeamCommand,
    },
    #[command(about = "Record human functional Task review using only configured user_name.")]
    Task {
        #[command(subcommand)]
        command: ReviewTaskCommand,
    },
    #[command(about = "Generate or submit structured AI code-review evidence attributed to the executing app.")]
    Code {
        #[command(subcommand)]
        command: ReviewCodeCommand,
    },
}

#[derive(Subcommand)]
enum ReviewTeamCommand {
    Request(ReviewRequestArgs),
    Approve(ReviewApproveArgs),
    RequestChanges(ReviewApproveArgs),
    Comment(ReviewApproveArgs),
    Defer(ReviewApproveArgs),
}

#[derive(Subcommand)]
enum ReviewTaskCommand {
    #[command(about = "Approve one specific Patchset after functional validation; task_review=required only.")]
    Approve(ReviewTaskApproveArgs),
    RequestChanges(ReviewTaskRecordArgs),
    Comment(ReviewTaskRecordArgs),
    Defer(ReviewTaskRecordArgs),
}

#[derive(Subcommand)]
enum ReviewCodeCommand {
    #[command(about = "Submit the implicit pass outcome for one exact, fully inspected Patchset.")]
    Submit(ReviewCodeSubmitArgs),
    #[command(about = "Print the required structured AI code-review summary template locally.")]
    Template(ReviewCodeTemplateArgs),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum ReviewCodeTemplateStyleArg {
    Inline,
    Numbered,
}

impl ReviewCodeTemplateStyleArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Numbered => "numbered",
        }
    }
}

#[derive(Subcommand)]
enum AttestCommand {
    #[command(about = "Record test, lint, security, license, and authorship evidence for a remote Patchset.")]
    Put(AttestPutArgs),
    #[command(about = "Show recorded evidence for one remote Patchset.")]
    Show(AttestShowArgs),
}

#[derive(Subcommand)]
enum PolicyCommand {
    #[command(about = "Check the configured Policy for a selected remote Patchset.")]
    Eval(PolicyEvalArgs),
    #[command(about = "Show the recorded Policy result for a remote Patchset.")]
    Show(PolicyShowArgs),
    #[command(about = "Request a reasoned Policy waiver for a remote Patchset.")]
    Waive(PolicyWaiveArgs),
}

#[derive(Subcommand)]
enum WorktreeCommand {
    #[command(
        about = "Compare one checkout with a Line head or specific Snapshot.",
        long_about = "Report modified, missing, and untracked paths in the selected checkout without restoring content or moving a Line head. NAME selects a registered worktree; when omitted, inspect the current checkout. --snapshot and --line are mutually exclusive, and omission of both compares with the current Line head."
    )]
    Status(WorktreeStatusArgs),
    #[command(
        about = "Restore all or selected checkout content from a Line head or Snapshot.",
        long_about = "Restore the selected registered worktree, or the current checkout when NAME is omitted. With no source option, restore the whole checkout from its current Line head. --snapshot and --line are mutually exclusive. Every --path is an exact workspace-relative path and requires an explicit --snapshot or --line; selected-path restore does not switch the current Line. Unsaved changes require --force, while --dry-run only reports the restore plan."
    )]
    Restore(WorktreeRestoreArgs),
    #[command(
        about = "Show one registered worktree and its refreshed live status.",
        long_about = "Show the registered and current Line, checked-out Snapshot, Task/Change binding, cleanup classification, and rebase or merge state for one worktree. NAME may be omitted only when the current checkout belongs to a registered worktree."
    )]
    Show(WorktreeShowArgs),
    #[command(
        visible_alias = "open",
        about = "Print one worktree path or a shell command that enters it.",
        long_about = "Print the managed alias path when available, otherwise the registered physical path. This command refreshes live status and records worktree use. --shell prints a command that changes directory and exports managed Cargo paths when enabled; the visible alias `open` has identical print-only behavior."
    )]
    Path(WorktreePathArgs),
    #[command(
        about = "Summarize registered worktree health and cleanup classification.",
        long_about = "Summarize current, clean, dirty, missing, detached, protected, and cleanup-candidate worktrees. The default uses metadata and cached status without a full content scan. --refresh verifies live content and may refresh derived runtime layout and status caches."
    )]
    Doctor(WorktreeDoctorArgs),
    #[command(
        name = "cleanup-candidates",
        about = "Inspect policy-selected cleanup candidates without removing them.",
        long_about = "Refresh and classify registered worktrees without removing paths or registrations. Filter by cleanup policy or idle age, opt clean manual-only worktrees into review, and optionally include protected rows with their exact reasons. Missing or detached registrations remain stale rather than cleanup candidates."
    )]
    CleanupCandidates(WorktreeCleanupCandidatesArgs),
    #[command(
        about = "Preview or remove policy-selected safe worktrees.",
        long_about = "Select safe cleanup candidates using the same policy rules as cleanup-candidates. --dry-run previews the exact ordered removal plan without requiring confirmation. Applied cleanup deletes selected worktree paths, managed aliases, registrations, and managed Task Cargo build caches and therefore requires --yes."
    )]
    Cleanup(WorktreeCleanupArgs),
    #[command(
        name = "prune-stale",
        about = "Preview or prune missing and detached worktree registrations.",
        long_about = "Select registrations whose path is missing or whose worktree runtime layout is detached. Pruning removes stale registration and alias state without deleting surviving checkout content. --dry-run previews the exact rows; applied pruning requires --yes."
    )]
    PruneStale(WorktreePruneStaleArgs),
    #[command(
        about = "List every registered worktree.",
        long_about = "List registered paths, current Lines, cached workspace status, cleanup classification, and current-context identity. --refresh verifies live content and may refresh derived runtime layout and status caches."
    )]
    List(WorktreeListArgs),
    #[command(
        about = "Synchronize one or all worktrees to their selected Line heads.",
        long_about = "Restore a complete registered worktree to a Line head and update its checked-out Snapshot, current Line, and runtime metadata. NAME selects one worktree; --all selects every available worktree and uses each one's current Line. --all cannot be combined with NAME or --line. Unsaved changes require --force, and --dry-run previews without applying the restore."
    )]
    Sync(WorktreeSyncArgs),
    #[command(
        about = "Recreate a missing registered Task worktree.",
        long_about = "Recreate the recorded path and alias of a Task-bound worktree whose registered path is missing. Recovery selects the first locally available current-Line head, fork Snapshot, or selected remote Patchset revision. An existing or unbound worktree is rejected; --dry-run validates and reports the candidate without creating its files."
    )]
    Recreate(WorktreeRecreateArgs),
    #[command(
        name = "recover-task",
        about = "Recover a local authoring worktree for an existing remote Task and Change.",
        long_about = "Run from the main repository root to validate an active or draft remote Task and its draft or review Change, then recreate their local feature Line and Task-bound worktree. This command does not create a remote Task or Change. If a registration already exists but its path is missing, use worktree recreate instead."
    )]
    RecoverTask(WorktreeRecoverTaskArgs),
    #[command(
        name = "restore-owned-head",
        about = "Restore the last contiguous Snapshot head owned by a bound Task worktree.",
        long_about = "Inspect first-parent history after the registered fork and retain the last contiguous Snapshot owned by the bound Task, Change, and worktree. Any first foreign Snapshot and its descendants are dropped from the Line head. The worktree must be Task-bound, clean, and outside a conflicted rebase; --dry-run reports the ownership decision without changing content or the head."
    )]
    RestoreOwnedHead(WorktreeRestoreOwnedHeadArgs),
    #[command(
        about = "Replay a worktree's feature delta onto a target base Line.",
        long_about = "Rebase the current feature Line from its registered fork onto --onto or the recorded target base Line. Starting an applied rebase requires a clean worktree. --dry-run previews ancestry, writes, removals, and conflicts. A conflicted rebase must later use exactly one of --continue or --abort; neither continuation mode accepts --dry-run."
    )]
    Rebase(WorktreeRebaseArgs),
    #[command(
        about = "Preview or explicitly remove registered worktrees.",
        long_about = "Remove one or more named worktree registrations, managed aliases, and runtime identity files, leaving ordinary checkout content unless --delete-path is supplied. --all-stale performs the same stale-registration pruning as prune-stale and cannot be combined with names, --delete-path, or --force. --dry-run previews the exact plan; every applied removal requires --yes."
    )]
    Remove(WorktreeRemoveArgs),
}

#[derive(Args, Clone)]
struct QueryScopeArgs {
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Use local Plans even when the workflow mode defaults to a remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        value_name = "NAME",
        help = "Use Plans from the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct PlanListArgs {
    #[command(flatten)]
    scope: QueryScopeArgs,
    #[arg(
        long,
        conflicts_with = "json",
        help = "Expand human output to include archived Plans; unavailable with --json"
    )]
    all: bool,
}

#[derive(Args, Clone)]
struct PlanIdArgs {
    #[arg(
        value_name = "PLAN",
        help = "Plan selector; local canonical forms are PR-<ordinal>, artifact:<path>, title:<title>, or published-plan:<index>"
    )]
    plan_id: String,
    #[command(flatten)]
    scope: QueryScopeArgs,
    #[arg(
        long,
        conflicts_with = "json",
        help = "Expand human output to the complete revision history; unavailable with --json"
    )]
    all: bool,
}

#[derive(Args, Clone)]
struct ShowArgs {
    #[arg(
        value_name = "PLAN",
        help = "Plan selector; local canonical forms are PR-<ordinal>, artifact:<path>, title:<title>, or published-plan:<index>"
    )]
    plan_id: String,
    #[arg(
        long,
        value_name = "REVISION",
        help = "Select a non-head revision; local canonical forms are plan-revision:<index>, revision-number:<number>, or published-revision:<index>"
    )]
    revision: Option<String>,
    #[command(flatten)]
    scope: QueryScopeArgs,
}

#[derive(Args, Clone)]
struct CandidatesArgs {
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Use local Plans even when the workflow mode defaults to a remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        value_name = "NAME",
        help = "Use Plans from the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(
        long = "all",
        help = "Also include active Plans with zero taskable items; archived Plans remain excluded"
    )]
    include_all: bool,
    #[arg(
        long,
        value_name = "TERMS",
        help = "Case-insensitive comma-delimited OR search across titles, paths, selectors, headings, item text, and item refs"
    )]
    contains: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct SyncArgs {
    #[arg(
        value_name = "TARGET",
        help = "Repository-relative Markdown file or directory to sync into local Plan revision history"
    )]
    target: PathBuf,
    #[arg(
        long,
        value_name = "PLAN_REF",
        help = "Select one exact [plan-ref: ...] root when one Markdown target contains multiple Plan roots"
    )]
    plan_ref: Option<String>,
    #[arg(
        long,
        help = "Archive tracked Plan artifacts missing from the complete selected target inventory"
    )]
    prune: bool,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Sync local Plan history only, even when the workflow mode defaults to a remote; never create a Snapshot or advance a Line"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        value_name = "NAME",
        help = "Sync local Plan history, then publish the updated heads to the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(
        long,
        conflicts_with = "reconcile",
        help = "When using a remote, replay the current local Plan head after the remote rejects divergent history"
    )]
    rebase: bool,
    #[arg(
        long,
        conflicts_with = "rebase",
        help = "Safely adopt verified divergent or mixed Plan identity before effective local reconciliation or remote publication"
    )]
    reconcile: bool,
    #[arg(
        long,
        help = "Emit the complete stable machine-readable result; failed and partial-success results still exit nonzero"
    )]
    json: bool,
}

#[derive(Subcommand)]
enum WorkflowCommand {
    #[command(
        about = "Show step-by-step guidance for common inventory and finish operations."
    )]
    Guide(WorkflowGuideArgs),
    #[command(
        about = "Inspect Task, Change, Line, worktree, land, and Plan bindings for repair; dry-run is the default and never changes Plan state."
    )]
    Reconcile(WorkflowReconcileArgs),
    #[command(
        about = "Check or prepare one Change for review and remote finish; changes require --apply."
    )]
    Ready(WorkflowReadyArgs),
    #[command(
        about = "Check or apply remote Review and Policy requirements for one Change's selected Patchset, then safely finish the ready Change."
    )]
    Finish(WorkflowFinishArgs),
}

#[derive(Args, Clone)]
struct TaskStartArgs {
    #[arg(
        long,
        help = "Unbound Task title; required and available only when sprint mode is off",
        required_unless_present = "source",
        conflicts_with = "source"
    )]
    title: Option<String>,
    #[arg(long, help = "Required Task intent in both manual and --from modes")]
    intent: String,
    #[arg(
        long = "from",
        value_name = "MARKDOWN_PATH#ITEM_REF",
        help = "Sprint-only exact file-backed Plan source; syncs, validates, binds, and derives the Task and initial Change title deterministically",
        conflicts_with = "title"
    )]
    source: Option<String>,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Start the Task locally even when the workflow mode defaults to a remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        help = "Start the Task on the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the compact versioned machine-readable Task-start result")]
    json: bool,
    #[arg(
        long,
        requires = "json",
        help = "With --json, emit the previous complete Task-start result"
    )]
    full: bool,
}

#[derive(Args, Clone)]
struct TaskListArgs {
    #[arg(
        long,
        conflicts_with = "remote",
        help = "List local Tasks even when the workflow mode defaults to a remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        help = "List Tasks from the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(long, help = "Show complete Task history instead of the bounded open view")]
    all: bool,
    #[arg(long, help = "Emit the selected Task inventory as machine-readable JSON")]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskShowArgs {
    task_id: String,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Read the local Task even when the workflow mode defaults to a remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        help = "Read the Task from the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete machine-readable Task record")]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskAuditArgs {
    task_id: String,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Check the local Task even when the workflow mode defaults to a remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        help = "Check the Task on the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete machine-readable audit and recommended action")]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskFinishArgs {
    #[arg(
        help = "Task ID or Change ID to finish onto main. The workflow mode chooses local or remote unless --local or --remote is provided."
    )]
    task_or_change_id: String,
    #[arg(
        long,
        value_name = "MESSAGE",
        help = "Create the final Snapshot from dirty local work before finishing; clean local work reuses the current Line head. Remote finish rejects this option."
    )]
    message: Option<String>,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Force local draft finish even when the configured workflow mode defaults to remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        help = "Force closeout through the named remote's already-ready selected Patchset even when the configured workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the compact versioned machine-readable finish and closeout result")]
    json: bool,
    #[arg(
        long,
        requires = "json",
        help = "With --json, emit the previous complete finish and closeout result"
    )]
    full: bool,
}

#[derive(Args, Clone)]
struct TaskAbandonArgs {
    task_id: String,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Abandon the local Task even when the workflow mode defaults to a remote"
    )]
    local: bool,
    #[arg(
        long,
        conflicts_with = "local",
        help = "Abandon the Task on the named remote even when the workflow mode defaults to local"
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete machine-readable terminal Task record")]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeCreateArgs {
    #[arg(help = "Existing Task ID that will own the new Change.")]
    task_id: String,
    #[arg(long, help = "Required title for the new Change.")]
    title: String,
    #[arg(
        long = "base-line",
        value_name = "LINE",
        help = "Base Line assertion; defaults to the bound worktree target Line, then the repository default Line."
    )]
    base_line: Option<String>,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Create the Change locally even when Change commands default to a remote."
    )]
    local: bool,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Create the Change on the named remote even when Change commands default to local."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit stable machine-readable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeListArgs {
    #[arg(
        long,
        conflicts_with = "remote",
        help = "List local Changes even when Change commands default to a remote."
    )]
    local: bool,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "List Changes from the named remote even when Change commands default to local."
    )]
    remote: Option<String>,
    #[arg(long, help = "Show complete Change history instead of the bounded open view.")]
    all: bool,
    #[arg(
        long,
        help = "Emit stable machine-readable JSON; the bounded view still applies unless --all is present."
    )]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeShowArgs {
    #[arg(
        value_name = "TASK_ID/C-##",
        help = "Task-owned Change reference; a bare C-## is accepted only when it identifies one Change."
    )]
    change_id: String,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Read the local Change even when Change commands default to a remote."
    )]
    local: bool,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Read the Change from the named remote even when Change commands default to local."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable Change payload.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeRevertArgs {
    #[arg(
        value_name = "TASK_ID/C-##",
        help = "Task-owned Change whose recorded delta will be removed from the workspace."
    )]
    change_id: String,
    #[arg(
        long,
        help = "Overwrite unsaved changes on paths selected by the Change delta."
    )]
    force: bool,
    #[arg(
        long = "dry-run",
        help = "Preview affected paths and overwrite risk without modifying the workspace."
    )]
    dry_run: bool,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Read the Change's local history even when Change commands default to a remote."
    )]
    local: bool,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Read the Change's history from the named remote; only local workspace files are changed."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable workspace plan or result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeReplayArgs {
    #[arg(
        value_name = "TASK_ID/C-##",
        help = "Task-owned Change whose recorded delta will be applied to the workspace."
    )]
    change_id: String,
    #[arg(
        long,
        value_name = "LINE",
        hide = true,
        help = "Compatibility assertion for the current Line; omitted uses the current Line."
    )]
    onto: Option<String>,
    #[arg(
        long,
        help = "Overwrite unsaved changes on paths selected by the Change delta."
    )]
    force: bool,
    #[arg(
        long = "dry-run",
        help = "Preview affected paths and overwrite risk without modifying the workspace."
    )]
    dry_run: bool,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Read the Change's local history even when Change commands default to a remote."
    )]
    local: bool,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Read the Change's history from the named remote; only local workspace files are changed."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable workspace plan or result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeCloseArgs {
    #[arg(
        value_name = "TASK_ID/C-##",
        help = "Task-owned Change to archive without landing."
    )]
    change_id: String,
    #[arg(
        long,
        conflicts_with = "remote",
        help = "Archive the local Change even when Change commands default to a remote."
    )]
    local: bool,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Archive the Change on the named remote even when Change commands default to local."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable closeout payload.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangePublishArgs {
    #[arg(
        value_name = "TASK_ID/C-##",
        help = "Local Task-owned draft Change to promote."
    )]
    change_id: String,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Destination remote; omitted uses the repository default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable publication payload.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotCreateArgs {
    #[arg(
        short = 'm',
        long,
        value_name = "MESSAGE",
        help = "Record an optional human-readable Snapshot message."
    )]
    message: Option<String>,
    #[arg(long, help = "Emit the compact versioned machine-readable creation payload.")]
    json: bool,
    #[arg(
        long,
        requires = "json",
        help = "With --json, emit the previous complete stable creation payload."
    )]
    full: bool,
}

#[derive(Args, Clone)]
struct SnapshotListArgs {
    #[arg(
        long,
        help = "Show complete Snapshot history instead of the bounded recent text or JSON view."
    )]
    all: bool,
    #[arg(long, help = "Emit the stable machine-readable Snapshot list.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotShowArgs {
    #[arg(
        value_name = "SNAPSHOT_OR_TAG",
        help = "Exact immutable Snapshot ID or local AIT Tag to inspect."
    )]
    snapshot_id: String,
    #[arg(
        long,
        help = "Show the complete Snapshot tree inventory in text output; JSON is always complete."
    )]
    files: bool,
    #[arg(long, help = "Emit the complete stable machine-readable Snapshot payload.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotDiffArgs {
    #[arg(
        value_name = "OLD_SNAPSHOT_OR_TAG",
        help = "Exact Snapshot ID or local AIT Tag for the old side."
    )]
    old_snapshot_id: String,
    #[arg(
        value_name = "NEW_SNAPSHOT_OR_TAG",
        help = "Exact Snapshot ID or local AIT Tag for the new side."
    )]
    new_snapshot_id: String,
    #[arg(
        long,
        help = "Include bounded line-oriented text diffs for eligible modified files."
    )]
    include_text: bool,
    #[arg(
        long,
        value_name = "BYTES",
        default_value_t = DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
        value_parser = parse_positive_usize,
        requires = "include_text",
        help = "Maximum old or new file size allowed for text diff generation; requires --include-text."
    )]
    max_bytes: usize,
    #[arg(long, help = "Emit the complete stable machine-readable diff payload.")]
    json: bool,
}

#[derive(Args, Clone)]
#[command(
    override_usage = "ait snapshot restore-lines <SNAPSHOT_ID> <PATH> (--line <N> | --start <N> --end <N>) [--yes] [--json]",
    group(
        ArgGroup::new("line_selection")
            .required(true)
            .multiple(false)
            .args(["line", "start_line"])
    )
)]
struct SnapshotRestoreLinesArgs {
    #[arg(help = "Exact immutable Snapshot ID to read line content from.")]
    snapshot_id: String,
    #[arg(help = "Existing regular workspace file to update.")]
    path: String,
    #[arg(
        long,
        value_parser = parse_positive_line,
        conflicts_with_all = ["start_line", "end_line"],
        help = "Select one 1-based workspace line."
    )]
    line: Option<usize>,
    #[arg(
        long = "start",
        value_parser = parse_positive_line,
        requires = "end_line",
        conflicts_with = "line",
        help = "Start of one inclusive 1-based line range."
    )]
    start_line: Option<usize>,
    #[arg(
        long = "end",
        value_parser = parse_positive_line,
        requires = "start_line",
        conflicts_with = "line",
        help = "End of one inclusive 1-based line range."
    )]
    end_line: Option<usize>,
    #[arg(
        long,
        help = "Apply the selected Snapshot lines; omission is always a read-only preview."
    )]
    yes: bool,
    #[arg(long, help = "Emit the complete stable machine-readable preview or apply result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotRevertArgs {
    #[arg(
        value_name = "SNAPSHOT_OR_TAG",
        help = "Current Line head Snapshot, identified by exact ID or local AIT Tag."
    )]
    snapshot_id: String,
    #[arg(
        long,
        help = "Overwrite unsaved changes on paths selected by the Snapshot delta."
    )]
    force: bool,
    #[arg(
        long = "dry-run",
        help = "Preview affected paths and overwrite risk without modifying the workspace."
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable workspace plan or result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotReplayArgs {
    #[arg(
        value_name = "SNAPSHOT_OR_TAG",
        help = "Non-root Snapshot whose recorded delta will be applied to the workspace."
    )]
    snapshot_id: String,
    #[arg(
        long,
        value_name = "LINE",
        hide = true,
        help = "Compatibility assertion for the current Line; omitted uses the current Line."
    )]
    onto: Option<String>,
    #[arg(
        long,
        help = "Overwrite unsaved changes on paths selected by the Snapshot delta."
    )]
    force: bool,
    #[arg(
        long = "dry-run",
        help = "Preview affected paths and overwrite risk without modifying the workspace."
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable workspace plan or result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotAncestryArgs {
    #[arg(
        value_name = "SNAPSHOT_OR_TAG",
        help = "Exact Snapshot ID or local AIT Tag at the query boundary."
    )]
    snapshot_id: String,
    #[arg(
        long,
        conflicts_with = "descendants",
        help = "Traverse ancestors; this is the default direction."
    )]
    ancestors: bool,
    #[arg(
        long,
        conflicts_with = "ancestors",
        help = "Traverse descendants instead of ancestors."
    )]
    descendants: bool,
    #[arg(
        long = "first-parent",
        help = "Follow only ordinal-zero parent edges instead of all parents."
    )]
    first_parent: bool,
    #[arg(
        long = "max-depth",
        value_name = "DEPTH",
        default_value_t = DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_MAX_DEPTH,
        value_parser = parse_positive_usize,
        help = "Stop traversal after this positive edge depth."
    )]
    max_depth: usize,
    #[arg(
        long,
        value_name = "COUNT",
        default_value_t = DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_LIMIT,
        value_parser = parse_positive_usize,
        help = "Admit at most this positive number of results."
    )]
    limit: usize,
    #[arg(
        long,
        help = "Show every result allowed by --limit in text output instead of the nearest 20; JSON already includes all of them."
    )]
    all: bool,
    #[arg(long, help = "Emit the complete stable machine-readable bounded query payload.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotIsAncestorArgs {
    #[arg(
        value_name = "OLDER_SNAPSHOT_OR_TAG",
        help = "Candidate ancestor, identified by exact Snapshot ID or local AIT Tag."
    )]
    older_snapshot_id: String,
    #[arg(
        value_name = "NEWER_SNAPSHOT_OR_TAG",
        help = "Candidate descendant, identified by exact Snapshot ID or local AIT Tag."
    )]
    newer_snapshot_id: String,
    #[arg(long, help = "Emit the stable machine-readable ancestry decision and distance.")]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotMergeBaseArgs {
    #[arg(
        value_name = "LEFT_SNAPSHOT_OR_TAG",
        help = "Left Snapshot, identified by exact ID or local AIT Tag."
    )]
    left_snapshot_id: String,
    #[arg(
        value_name = "RIGHT_SNAPSHOT_OR_TAG",
        help = "Right Snapshot, identified by exact ID or local AIT Tag."
    )]
    right_snapshot_id: String,
    #[arg(long, help = "Emit every equally best common ancestor in deterministic order.")]
    all: bool,
    #[arg(long, help = "Emit the complete stable machine-readable merge-base result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct StashSaveArgs {
    #[arg(long, help = "Record an optional human-readable message with the stash.")]
    message: Option<String>,
    #[arg(
        long = "keep-workspace",
        help = "Leave the saved files in the workspace; the Line head stays unchanged, so the workspace remains dirty relative to it."
    )]
    keep_workspace: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct StashListArgs {
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct StashIdArgs {
    #[arg(value_name = "STASH_ID", help = "Exact active stash ID.")]
    stash_id: String,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct StashRestoreArgs {
    #[arg(
        value_name = "STASH_ID",
        help = "Exact active stash ID created on the current Line."
    )]
    stash_id: String,
    #[arg(
        long,
        help = "Overwrite unsaved managed-workspace changes; this does not permit restoring a stash from another Line."
    )]
    force: bool,
    #[arg(long, help = "Emit the complete machine-readable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct TagCreateArgs {
    #[arg(
        value_name = "NAME",
        help = "New local Tag name; an existing name is rejected."
    )]
    name: String,
    #[arg(
        long,
        value_name = "SNAPSHOT_ID",
        help = "Exact existing local Snapshot ID; defaults to the current Line head."
    )]
    snapshot: Option<String>,
    #[arg(
        long,
        value_name = "MESSAGE",
        help = "Required non-empty, single-line reason for creating the Tag."
    )]
    message: String,
    #[arg(long, help = "Emit the created Tag as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct TagListArgs {
    #[arg(long, help = "Emit the complete local Tag list as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct TagShowArgs {
    #[arg(value_name = "NAME", help = "Exact local Tag name to inspect.")]
    name: String,
    #[arg(long, help = "Emit the Tag record as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct TagDeleteArgs {
    #[arg(value_name = "NAME", help = "Exact local Tag name to delete.")]
    name: String,
    #[arg(long, help = "Emit the deleted Tag record as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetPublishArgs {
    #[arg(
        value_name = "CHANGE_ID",
        help = "Exact remote Change ID that will own the new Patchset."
    )]
    change: String,
    #[arg(
        long,
        value_name = "SUMMARY",
        help = "Required human-readable summary of this published revision."
    )]
    summary: String,
    #[arg(
        long = "author-mode",
        value_enum,
        value_name = "MODE",
        help = "Override provenance mode for this publication; otherwise use configured default_author_mode."
    )]
    author_mode: Option<ConfigAuthorModeArg>,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Configured remote name; defaults to the repository's default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete publication result as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetListArgs {
    #[arg(
        value_name = "CHANGE_ID",
        help = "Exact remote Change ID whose published Patchsets will be listed."
    )]
    change: String,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Configured remote name; defaults to the repository's default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete Patchset list as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetShowArgs {
    #[arg(
        value_name = "PATCHSET_ID",
        value_parser = parse_exact_patchset_id,
        help = "Complete published Patchset ID; bare numeric ordinals are rejected."
    )]
    patchset_id: String,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Configured remote name; defaults to the repository's default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete Patchset record as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetSelectArgs {
    #[arg(
        value_name = "PATCHSET_ID",
        value_parser = parse_exact_patchset_id,
        help = "Complete published Patchset ID; its owning Change is derived remotely."
    )]
    patchset_id: String,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Configured remote name; defaults to the repository's default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete selection result as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetCiStatusArgs {
    #[arg(
        value_name = "PATCHSET_ID",
        value_parser = parse_exact_patchset_id,
        help = "Complete published Patchset ID; bare numeric ordinals are rejected."
    )]
    patchset_id: String,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Configured remote name; defaults to the repository's default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete CI status and fixed recent-job window as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetRerunCiArgs {
    #[arg(
        value_name = "PATCHSET_ID",
        value_parser = parse_exact_patchset_id,
        help = "Complete published Patchset ID to enqueue for a manual rerun."
    )]
    patchset_id: String,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Configured remote name; defaults to the repository's default remote."
    )]
    remote: Option<String>,
    #[arg(long, help = "Emit the complete queued-run result as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewApproveArgs {
    change_id: String,
    #[arg(long)]
    reviewer: Option<String>,
    #[arg(long = "patchset")]
    patchset_id: Option<String>,
    #[arg(long)]
    message: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewRequestArgs {
    change_id: String,
    #[arg(long = "group", required = true)]
    reviewer_groups: Vec<String>,
    #[arg(long = "patchset")]
    patchset_id: Option<String>,
    #[arg(long)]
    note: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewCodeSubmitArgs {
    #[arg(help = "Remote Change that owns the exact reviewed Patchset.")]
    change_id: String,
    #[arg(
        long = "patchset",
        value_parser = parse_exact_patchset_id,
        help = "Complete published Patchset ID reviewed by the executing AI app; numeric Repository references are rejected."
    )]
    patchset_id: String,
    #[arg(
        long,
        help = "Structured Reviewed files, Findings, Risks, Tests, and pass Recommendation summary for this specific Patchset."
    )]
    message: String,
    #[arg(long, help = "Configured remote name; defaults to the repository's default remote.")]
    remote: Option<String>,
    #[arg(long, help = "Emit both code and Task review-lane results as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewTaskApproveArgs {
    #[arg(help = "Remote Change that owns the functionally validated Patchset.")]
    change_id: String,
    #[arg(
        long = "patchset",
        value_parser = parse_exact_patchset_id,
        help = "Complete published Patchset ID whose functionality was validated; numeric Repository references are rejected."
    )]
    patchset_id: String,
    #[arg(
        long,
        help = "Non-empty functional-validation evidence from the configured user_name."
    )]
    message: String,
    #[arg(long, help = "Configured remote name; defaults to the repository's default remote.")]
    remote: Option<String>,
    #[arg(long, help = "Emit the recorded Task approval as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewTaskRecordArgs {
    change_id: String,
    #[arg(long = "patchset")]
    patchset_id: Option<String>,
    #[arg(long)]
    message: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewCodeTemplateArgs {
    #[arg(
        long,
        value_enum,
        default_value = "numbered",
        help = "Template layout: inline or numbered."
    )]
    style: ReviewCodeTemplateStyleArg,
    #[arg(long, help = "Emit the template metadata as stable JSON.")]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewShowArgs {
    #[arg(help = "Remote Change ID or supported Change reference to inspect.")]
    change_id: String,
    #[arg(long, help = "Configured remote name; defaults to the repository's default remote.")]
    remote: Option<String>,
    #[arg(long, help = "Emit the compact stable machine-readable Review result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct AttestPutArgs {
    patchset_id: Option<String>,
    #[arg(long)]
    change: Option<String>,
    #[arg(long)]
    tests: Option<String>,
    #[arg(long)]
    lint: Option<String>,
    #[arg(long)]
    security: Option<String>,
    #[arg(long)]
    license: Option<String>,
    #[arg(long = "author-mode")]
    author_mode: Option<String>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct AttestShowArgs {
    patchset_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PolicyEvalArgs {
    patchset_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PolicyShowArgs {
    patchset_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PolicyWaiveArgs {
    patchset_id: String,
    #[arg(long = "rule")]
    rule_name: String,
    #[arg(long)]
    reason: String,
    #[arg(long = "expires-at")]
    expires_at: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeStatusArgs {
    #[arg(
        value_name = "NAME",
        help = "Registered worktree name; omit it to inspect the current checkout"
    )]
    name: Option<String>,
    #[arg(
        long = "snapshot",
        value_name = "SNAPSHOT_ID",
        help = "Compare with this exact local Snapshot instead of a Line head"
    )]
    snapshot_id: Option<String>,
    #[arg(
        long = "line",
        value_name = "LINE_NAME",
        help = "Compare with this local Line head instead of the current Line head"
    )]
    line_name: Option<String>,
    #[arg(long, help = "Show baseline and workspace-root detail in text output")]
    verbose: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRestoreArgs {
    #[arg(
        value_name = "NAME",
        help = "Registered worktree name; omit it to restore the current checkout"
    )]
    name: Option<String>,
    #[arg(
        long = "snapshot",
        value_name = "SNAPSHOT_ID",
        help = "Restore from this exact local Snapshot; mutually exclusive with --line"
    )]
    snapshot_id: Option<String>,
    #[arg(
        long = "line",
        value_name = "LINE_NAME",
        help = "Restore from this local Line head; a whole-checkout restore also switches the current Line"
    )]
    line_name: Option<String>,
    #[arg(
        long = "path",
        value_name = "PATH",
        help = "Restore one exact workspace-relative path; repeat for more paths and supply --snapshot or --line"
    )]
    paths: Vec<String>,
    #[arg(
        long,
        help = "Overwrite unsaved changes in the selected files"
    )]
    force: bool,
    #[arg(
        long = "dry-run",
        help = "Report writes, removals, and overwritten changes without restoring content"
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeShowArgs {
    #[arg(
        value_name = "NAME",
        help = "Registered worktree name; omit it to use the current runtime worktree binding"
    )]
    name: Option<String>,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreePathArgs {
    #[arg(
        value_name = "NAME",
        help = "Registered worktree name; omit it to use the current runtime worktree binding"
    )]
    name: Option<String>,
    #[arg(
        long = "shell",
        help = "Print a shell command that enters the worktree and exports managed Cargo paths"
    )]
    shell_output: bool,
    #[arg(
        long,
        help = "Emit machine-readable path, command, status, and managed runtime fields"
    )]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeDoctorArgs {
    #[arg(
        long,
        help = "Verify each worktree's live content and refresh derived runtime status before reporting"
    )]
    refresh: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeCleanupCandidatesArgs {
    #[arg(
        long = "older-than",
        value_name = "DURATION",
        default_value = "7d",
        help = "Idle threshold for after_idle policy, written as integer days, hours, or minutes (for example 7d, 12h, or 30m)"
    )]
    older_than: String,
    #[arg(
        long = "policy",
        value_name = "POLICY",
        help = "Filter by cleanup policy: manual_only, after_remote_land, after_task_complete, after_idle, or never"
    )]
    cleanup_policy: Option<String>,
    #[arg(
        long = "allow-manual-only",
        help = "Classify otherwise-safe clean manual_only worktrees as explicit cleanup candidates"
    )]
    allow_manual_only: bool,
    #[arg(
        long = "include-protected",
        help = "Include protected worktree rows and their reasons without making them removable"
    )]
    include_protected: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeCleanupArgs {
    #[arg(
        long = "older-than",
        value_name = "DURATION",
        default_value = "7d",
        help = "Idle threshold for after_idle policy, written as integer days, hours, or minutes (for example 7d, 12h, or 30m)"
    )]
    older_than: String,
    #[arg(
        long = "policy",
        value_name = "POLICY",
        help = "Filter by cleanup policy: manual_only, after_remote_land, after_task_complete, after_idle, or never"
    )]
    cleanup_policy: Option<String>,
    #[arg(
        long = "allow-manual-only",
        help = "Permit otherwise-safe clean manual_only worktrees to be selected for removal"
    )]
    allow_manual_only: bool,
    #[arg(
        long,
        value_name = "COUNT",
        help = "Remove at most this many ordered candidates"
    )]
    limit: Option<usize>,
    #[arg(
        long = "dry-run",
        help = "Preview the exact ordered removals without changing paths or registrations"
    )]
    dry_run: bool,
    #[arg(
        long,
        help = "Confirm and apply cleanup removal; required unless --dry-run is supplied"
    )]
    yes: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreePruneStaleArgs {
    #[arg(
        long = "dry-run",
        help = "Preview missing and detached registrations without pruning them"
    )]
    dry_run: bool,
    #[arg(
        long,
        help = "Confirm and apply stale-registration pruning; required unless --dry-run is supplied"
    )]
    yes: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeListArgs {
    #[arg(long, help = "Emit the complete stable machine-readable JSON array")]
    json: bool,
    #[arg(
        long,
        help = "Verify live content and refresh derived runtime status for every worktree"
    )]
    refresh: bool,
}

#[derive(Args, Clone)]
struct WorktreeSyncArgs {
    #[arg(
        value_name = "NAME",
        help = "Registered worktree name; omit it to use the current runtime worktree binding"
    )]
    name: Option<String>,
    #[arg(
        long = "all",
        help = "Synchronize every live worktree to its own current Line; cannot be combined with NAME or --line"
    )]
    all_worktrees: bool,
    #[arg(
        long = "line",
        value_name = "LINE_NAME",
        help = "Synchronize one worktree to this Line head and make that Line current"
    )]
    line_name: Option<String>,
    #[arg(long, help = "Overwrite unsaved changes while synchronizing")]
    force: bool,
    #[arg(
        long = "dry-run",
        help = "Preview complete restore plans without synchronizing content or metadata"
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRecreateArgs {
    #[arg(
        value_name = "NAME",
        help = "Missing registered Task worktree name; omit it to use the current runtime binding"
    )]
    name: Option<String>,
    #[arg(
        long = "dry-run",
        help = "Validate recovery candidates, path, and alias without recreating the worktree"
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRecoverTaskArgs {
    #[arg(value_name = "TASK_ID", help = "Existing remote Task ID to recover")]
    task_id: String,
    #[arg(
        long,
        value_name = "CHANGE",
        help = "Existing remote Change ID or Task-owned Change reference"
    )]
    change: String,
    #[arg(
        long,
        value_name = "REMOTE",
        help = "Configured remote name; omit it to use the effective default remote"
    )]
    remote: Option<String>,
    #[arg(
        long = "dry-run",
        help = "Validate remote identity, local Snapshots, and placement without creating a worktree"
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRestoreOwnedHeadArgs {
    #[arg(
        value_name = "NAME",
        help = "Task-bound worktree name; omit it to use the current runtime worktree binding"
    )]
    name: Option<String>,
    #[arg(
        long = "dry-run",
        help = "Report the retained owned head and dropped foreign Snapshots without restoring"
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRebaseArgs {
    #[arg(
        value_name = "NAME",
        help = "Registered worktree name; omit it to use the current runtime worktree binding"
    )]
    name: Option<String>,
    #[arg(
        long = "onto",
        value_name = "LINE_NAME",
        help = "Target base Line for a new rebase; omit it to use recorded target metadata"
    )]
    onto_line: Option<String>,
    #[arg(
        long = "continue",
        help = "Snapshot the resolved conflicted workspace and complete its rebase"
    )]
    continue_rebase: bool,
    #[arg(
        long = "abort",
        help = "Discard conflicted workspace resolution and restore the original head"
    )]
    abort_rebase: bool,
    #[arg(
        long = "dry-run",
        help = "Preview a new rebase plan; cannot be combined with --continue or --abort"
    )]
    dry_run: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRemoveArgs {
    #[arg(
        value_name = "NAME",
        help = "One or more registered worktree names; mutually exclusive with --all-stale"
    )]
    names: Vec<String>,
    #[arg(
        long = "all-stale",
        help = "Prune every missing or detached registration; cannot be combined with names, --delete-path, or --force"
    )]
    all_stale: bool,
    #[arg(
        long = "delete-path",
        help = "Also delete each physical worktree path and managed Task Cargo build cache"
    )]
    delete_path: bool,
    #[arg(
        long,
        help = "Permit explicit removal of dirty worktrees; does not bypass other safety checks"
    )]
    force: bool,
    #[arg(
        long = "dry-run",
        help = "Preview exact removals without changing paths, aliases, or registrations"
    )]
    dry_run: bool,
    #[arg(
        long,
        help = "Confirm and apply worktree removal; required unless --dry-run is supplied"
    )]
    yes: bool,
    #[arg(long, help = "Emit the complete stable machine-readable JSON payload")]
    json: bool,
}

#[derive(Args, Clone)]
struct WorkflowGuideArgs {
    topic: Option<String>,
}

#[derive(Args, Clone)]
struct WorkflowReconcileArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    task: Option<String>,
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    apply: bool,
    #[arg(long = "safe-only")]
    safe_only: bool,
    #[arg(
        long,
        requires = "apply",
        conflicts_with = "dry_run",
        help = "Run one bounded remote-worker reconciliation pass; implies --safe-only and requires a selected or default remote"
    )]
    scheduled: bool,
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorkflowReadyArgs {
    change_id: String,
    #[arg(long)]
    apply: bool,
    #[arg(long = "snapshot-message", requires = "apply")]
    snapshot_message: Option<String>,
    #[arg(long, requires = "apply")]
    summary: Option<String>,
    #[arg(long, requires = "apply")]
    tests: Option<String>,
    #[arg(long, requires = "apply")]
    lint: Option<String>,
    #[arg(long, requires = "apply")]
    security: Option<String>,
    #[arg(long, requires = "apply")]
    license: Option<String>,
    #[arg(long = "author-mode", requires = "apply")]
    author_mode: Option<ConfigAuthorModeArg>,
    #[arg(long, requires = "apply")]
    model: Option<String>,
    #[arg(long)]
    remote: Option<String>,
}

#[derive(Args, Clone)]
struct WorkflowFinishArgs {
    #[arg(help = "Remote Change id to inspect or finish through reviewer-owned closeout.")]
    change_id: String,
    #[arg(
        long,
        help = "Apply the safe next reviewer, policy, and atomic closeout actions instead of only showing state."
    )]
    apply: bool,
    #[arg(
        long = "review-message",
        requires = "apply",
        help = "Structured exact-Patchset AI review summary authored by the executing reviewer app."
    )]
    review_message: Option<String>,
    #[arg(long, help = "Use the named remote instead of the configured default remote.")]
    remote: Option<String>,
}

fn parse_positive_line(value: &str) -> Result<usize, String> {
    let line = value
        .parse::<usize>()
        .map_err(|_| "line number must be a positive integer".to_string())?;
    if line == 0 {
        return Err("line number must be 1 or greater".to_string());
    }
    Ok(line)
}

fn parse_positive_usize(value: &str) -> Result<usize, String> {
    let number = value
        .parse::<usize>()
        .map_err(|_| "value must be a positive integer".to_string())?;
    if number == 0 {
        return Err("value must be 1 or greater".to_string());
    }
    Ok(number)
}

fn parse_exact_patchset_id(value: &str) -> Result<String, String> {
    let patchset_id = value.trim();
    if patchset_id.is_empty() {
        return Err("Patchset ID must be non-empty".to_string());
    }
    if patchset_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(
            "complete published Patchset ID required; numeric Repository references are ambiguous"
                .to_string(),
        );
    }
    Ok(patchset_id.to_string())
}
