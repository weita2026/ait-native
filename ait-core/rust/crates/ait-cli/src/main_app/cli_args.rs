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
    Install(InstallArgs),
    #[command(hide = true)]
    BinaryDb {
        #[command(subcommand)]
        command: BinaryDbCommand,
    },
    Agent {
        #[command(subcommand)]
        command: ait_cli::agent_surface::AgentCommand,
    },
    Line {
        #[command(subcommand)]
        command: LineCommand,
    },
    Git {
        #[command(subcommand)]
        command: GitCommand,
    },
    Blame(BlameArgs),
    Doctor {
        #[command(subcommand)]
        command: DoctorCommand,
    },
    Queue {
        #[command(subcommand)]
        command: QueueCommand,
    },
    Remote {
        #[command(subcommand)]
        command: RemoteCommand,
    },
    #[command(hide = true)]
    CurrentSourceCache {
        #[command(subcommand)]
        command: CurrentSourceCacheCommand,
    },
    Release {
        #[command(subcommand)]
        command: ReleaseCommand,
    },
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Test {
        #[command(subcommand)]
        command: TestCommand,
    },
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    External {
        #[command(subcommand)]
        command: ExternalCommand,
    },
    Status(StatusArgs),
    Diff(DiffArgs),
    Pull(PullArgs),
    Push(PushArgs),
    Gc {
        #[command(subcommand)]
        command: GcCommand,
    },
    Stash {
        #[command(subcommand)]
        command: StashCommand,
    },
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    Change {
        #[command(subcommand)]
        command: ChangeCommand,
    },
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    Tag {
        #[command(subcommand)]
        command: TagCommand,
    },
    Patchset {
        #[command(subcommand)]
        command: PatchsetCommand,
    },
    Review {
        #[command(subcommand)]
        command: ReviewCommand,
    },
    Attest {
        #[command(subcommand)]
        command: AttestCommand,
    },
    Policy {
        #[command(subcommand)]
        command: PolicyCommand,
    },
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
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
    #[arg(long, help = "Repository name used only when creating missing authority.")]
    name: Option<String>,
    #[arg(
        long = "default-line",
        default_value = "main",
        help = "Initial Line name used only when creating missing authority."
    )]
    default_line: String,
    #[arg(
        long = "policy-profile",
        default_value = "prototype",
        help = "Initial policy profile: prototype, team, or release."
    )]
    policy_profile: String,
    #[arg(
        long = "default-author-mode",
        default_value = "ai_with_human_review",
        help = "Initial author mode used only when creating missing authority."
    )]
    default_author_mode: String,
    #[arg(
        long = "default-model",
        help = "Optional initial model; reinitialization preserves existing config."
    )]
    default_model: Option<String>,
    #[arg(
        long = "repair-existing",
        help = "Complete missing structure in an existing .ait directory; malformed authority is never overwritten."
    )]
    repair_existing: bool,
    #[arg(long, help = "Emit the stable JSON result.")]
    json: bool,
}

#[derive(Args, Clone)]
struct InstallArgs {
    #[arg(long, help = "Workflow mode choice: local, remote, solo_local, or solo_remote.")]
    mode: Option<String>,
    #[arg(
        long,
        help = "Optional transport attach: none, telegram, discord, or both."
    )]
    attach: Option<String>,
    #[arg(
        long = "server-setup",
        help = "Remote-backed ait-server setup: skip, connect, or deploy (guidance only; no infrastructure is deployed)."
    )]
    server_setup: Option<String>,
    #[arg(
        long = "server-url",
        help = "ait-server URL when --server-setup connect is selected."
    )]
    server_url: Option<String>,
    #[arg(
        long = "remote-name",
        default_value = "origin",
        help = "Remote name to use when connecting an ait-server."
    )]
    remote_name: String,
    #[arg(
        long = "remote-repo-name",
        help = "Remote repository name to associate with the ait-server."
    )]
    remote_repo_name: Option<String>,
    #[arg(
        long = "name",
        help = "Repository name to use if this command initializes the current directory."
    )]
    repo_name: Option<String>,
    #[arg(long = "user-name", help = "User name to store in ait config.")]
    user_name: Option<String>,
    #[arg(long = "user-email", help = "User email to store in ait config.")]
    user_email: Option<String>,
    #[arg(
        long = "init",
        action = ArgAction::SetTrue,
        conflicts_with = "no_init",
        help = "Initialize the current directory when no ait repository exists yet."
    )]
    init: bool,
    #[arg(
        long = "no-init",
        action = ArgAction::SetTrue,
        help = "Do not initialize when no ait repository exists."
    )]
    no_init: bool,
    #[arg(
        long = "sprint",
        action = ArgAction::SetTrue,
        conflicts_with = "no_sprint",
        help = "Enable sprint-style required plan item binding."
    )]
    sprint: bool,
    #[arg(
        long = "no-sprint",
        action = ArgAction::SetTrue,
        help = "Disable sprint-style plan item binding."
    )]
    no_sprint: bool,
    #[arg(
        long = "worker-name",
        default_value = "main",
        help = "Worker name to use for attached transports."
    )]
    worker_name: String,
    #[arg(
        long = "telegram-token",
        help = "Telegram bot token; prefer AIT_TELEGRAM_BOT_TOKEN or the hidden interactive prompt to avoid shell history."
    )]
    telegram_token: Option<String>,
    #[arg(long = "telegram-username", help = "Telegram bot username.")]
    telegram_username: Option<String>,
    #[arg(
        long = "discord-application-id",
        help = "Discord application id; may also be read from AIT_DISCORD_APPLICATION_ID."
    )]
    discord_application_id: Option<String>,
    #[arg(
        long = "discord-bot-token",
        help = "Discord bot token; prefer AIT_DISCORD_BOT_TOKEN or the hidden interactive prompt to avoid shell history."
    )]
    discord_bot_token: Option<String>,
    #[arg(
        long = "dry-run",
        help = "Preview detected state and planned changes without writing config."
    )]
    dry_run: bool,
    #[arg(
        long,
        help = "Emit JSON without prompts; omitted workflow and sprint choices preserve an existing repository or use fresh-repository defaults."
    )]
    json: bool,
}

#[derive(Subcommand)]
enum GcCommand {
    Stats(GcStatsArgs),
    Validate(JsonOnlyArgs),
    #[command(about = "Remove fully orphaned object packs while preserving every referenced payload")]
    Prune(JsonOnlyArgs),
}

#[derive(Args, Clone)]
struct GcStatsArgs {
    #[arg(
        long,
        help = "Compute exact snapshot reachability and validation fields. This scans retained tree payloads."
    )]
    deep: bool,
    #[arg(
        long = "include-inventory",
        help = "Include full object-pack and tree-pack inventory rows and exact reachability in addition to the bounded summary."
    )]
    include_inventory: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct JsonOnlyArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum LineCommand {
    List(LineListArgs),
    Create(LineCreateArgs),
    Switch(LineSwitchArgs),
    Show(LineShowArgs),
    Archive(LineArchiveArgs),
    #[command(about = "Rename a line while preserving its stable identity and reconciling bound pointers.")]
    Rename(LineRenameArgs),
    #[command(about = "Delete only a line ref after binding and unique-history protection checks.")]
    Delete(LineDeleteArgs),
    #[command(about = "Merge one line into the current target with resumable conflict state and a two-parent Snapshot.")]
    Merge(LineMergeArgs),
    #[command(name = "cleanup-candidates")]
    CleanupCandidates(LineCleanupCandidatesArgs),
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
    source: String,
    #[arg(long = "all-refs")]
    all_refs: bool,
    #[arg(long, conflicts_with = "resume")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    resume: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct GitExportArgs {
    target: String,
    #[arg(long = "all-refs")]
    all_refs: bool,
    #[arg(long, conflicts_with = "resume")]
    dry_run: bool,
    #[arg(long, conflicts_with = "dry_run")]
    resume: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct GitMirrorArgs {
    endpoint: String,
    #[arg(long, value_parser = ["inbound", "outbound", "bidirectional"])]
    direction: String,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    once: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum QueueCommand {
    Summary(QueueSummaryArgs),
}

#[derive(Subcommand)]
enum RemoteCommand {
    Add(RemoteAddArgs),
    List(RemoteListArgs),
    #[command(name = "recover-head")]
    RecoverHead(RemoteRecoverHeadArgs),
}

#[derive(Subcommand)]
enum ExternalCommand {
    Update(ExternalUpdateArgs),
    Status(JsonOnlyArgs),
    Doctor(JsonOnlyArgs),
    Link(ExternalLinkArgs),
    Unlink(ExternalUnlinkArgs),
}

#[derive(Args, Clone)]
struct ExternalUpdateArgs {
    name: Option<String>,
    #[arg(long = "to")]
    snapshot: Option<String>,
    #[arg(long)]
    latest: bool,
    #[arg(long)]
    locked: bool,
    #[arg(long)]
    validate: bool,
    #[arg(long = "no-recursive")]
    no_recursive: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ExternalLinkArgs {
    name: String,
    path: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ExternalUnlinkArgs {
    name: String,
    #[arg(long)]
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
    Show(RemoteJsonArgs),
    Retire(RepoRetireArgs),
    Restore(RemoteJsonArgs),
    Jobs(RepoJobsArgs),
    #[command(name = "run-ci")]
    RunCi(RepoRunCiArgs),
    #[command(name = "ci-capabilities")]
    CiCapabilities(RemoteJsonArgs),
    #[command(name = "ci-runs")]
    CiRuns(RepoCiRunsArgs),
}

#[derive(Subcommand)]
enum TestCommand {
    Run(TestRunArgs),
    Status(TestStatusArgs),
    #[command(hide = true, name = "patchset-ci")]
    PatchsetCi {
        #[command(subcommand)]
        command: PatchsetCiSmokeCommand,
    },
}

#[derive(Subcommand)]
enum PatchsetCiSmokeCommand {
    Preflight(JsonOnlyArgs),
    #[command(name = "package-smoke")]
    PackageSmoke(JsonOnlyArgs),
    #[command(name = "stable-smoke")]
    StableSmoke(JsonOnlyArgs),
    #[command(name = "release-artifact-smoke")]
    ReleaseArtifactSmoke(JsonOnlyArgs),
    #[command(name = "tg1-required")]
    Tg1Required(PatchsetCiTg1Args),
}

#[derive(Args, Clone)]
struct PatchsetCiTg1Args {
    #[arg(long)]
    json: bool,
    #[arg()]
    case_ids: Vec<String>,
}

#[derive(Subcommand)]
enum AuthCommand {
    Whoami(AuthWhoamiArgs),
    Grant(AuthGrantArgs),
    Bindings(AuthBindingsArgs),
}

#[derive(Subcommand)]
enum ConfigCommand {
    Show(ConfigShowArgs),
    Set(Box<ConfigSetArgs>),
}

#[derive(Subcommand)]
enum DoctorCommand {
    MemoryRoot(DoctorMemoryRootArgs),
    RuntimeRoot(DoctorRuntimeRootArgs),
    Postgres(DoctorPostgresArgs),
    PlanAuthority(DoctorPlanAuthorityArgs),
    PlanAuthorityWheel(DoctorPlanAuthorityWheelArgs),
}

#[derive(Args, Clone)]
struct LineListArgs {
    #[arg(long = "all")]
    include_all: bool,
    #[arg(long)]
    archived: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineCreateArgs {
    name: String,
    #[arg(long = "from-snapshot")]
    from_snapshot: Option<String>,
    #[arg(long)]
    switch: bool,
    #[arg(long)]
    restore: bool,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineSwitchArgs {
    name: String,
    #[arg(long)]
    restore: bool,
    #[arg(long)]
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
    #[arg(long = "into", conflicts_with_all = ["continue_merge", "abort_merge"])]
    target: Option<String>,
    #[arg(long, conflicts_with = "abort_merge")]
    message: Option<String>,
    #[arg(long = "continue", conflicts_with_all = ["source", "target", "abort_merge"])]
    continue_merge: bool,
    #[arg(long = "abort", conflicts_with_all = ["source", "target", "message", "continue_merge"])]
    abort_merge: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineCleanupCandidatesArgs {
    #[arg(long = "older-than", default_value = "7d")]
    older_than: String,
    #[arg(long = "kind")]
    cleanup_kind: Option<String>,
    #[arg(long = "include-protected")]
    include_protected: bool,
    #[arg(
        long,
        help = "Show every selected row; protected rows still require --include-protected"
    )]
    all: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct LineCleanupArgs {
    #[arg(long = "older-than", default_value = "7d")]
    older_than: String,
    #[arg(long = "kind")]
    cleanup_kind: Option<String>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct QueueSummaryArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, default_value = "active")]
    status: String,
    #[arg(long = "all-changes")]
    all_changes: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteAddArgs {
    name: String,
    url: String,
    #[arg(long = "repo-name")]
    repo_name: Option<String>,
    #[arg(long = "default", help = "Mark as default push/pull remote")]
    default: bool,
    #[arg(
        long = "discard-export",
        help = "Explicitly discard this remote's existing local retirement archive before fresh registration"
    )]
    discard_export: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteListArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteRecoverHeadArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    line: Option<String>,
    #[arg(
        long = "include-line",
        help = "Also recover this remote line head into the same atomic generation"
    )]
    include_lines: Vec<String>,
    #[arg(long, default_value_t = 8)]
    jobs: usize,
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    json: bool,
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
        help = "Protected CI only: retain exported public Git source authority while verifying family receipts"
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
        help = "Freeze family artifacts admitted from component receipts below DIR"
    )]
    receipts: Option<PathBuf>,
    #[arg(
        long = "public-source-root",
        value_name = "DIR",
        hide = true,
        help = "Protected CI only: retain exported public Git source authority while freezing family artifacts"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(
        long = "native-matrix-dir",
        value_name = "DIR",
        help = "Read target artifacts from DIR/<rust-target-triple>/release using ait-native-source.json descriptors."
    )]
    native_matrix_dir: Option<PathBuf>,
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
        help = "Protected CI only: retain exported public Git source authority while assembling a frozen family channel"
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
        help = "Protected CI only: retain exported public Git source authority while inspecting a family dossier"
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
        help = "Protected CI only: retain exported public Git source authority while emitting the family promotion handoff"
    )]
    public_source_root: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RemoteJsonArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RepoRetireArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(
        long,
        conflicts_with = "replace_export",
        help = "Abort an in-progress retirement and reactivate the Repository"
    )]
    abort: bool,
    #[arg(
        long = "replace-export",
        help = "Explicitly replace an unrelated complete local retirement archive"
    )]
    replace_export: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RepoJobsArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long = "worker-job-index")]
    worker_job_index: Option<u32>,
    #[arg(long)]
    state: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: u32,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RepoRunCiArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long = "suite")]
    suite_ids: Vec<String>,
    #[arg(long)]
    plane: Option<String>,
    #[arg(long = "target-line", default_value = "main")]
    target_line: String,
    #[arg(long, default_value = "manual_rerun")]
    trigger: String,
    #[arg(long)]
    selector: Option<String>,
    #[arg(long = "task-id")]
    task_ids: Vec<String>,
    #[arg(long = "curated-corpus")]
    curated_corpus: Option<String>,
    #[arg(long)]
    count: Option<i64>,
    #[arg(long = "window-days")]
    window_days: Option<i64>,
    #[arg(long = "dependency-evidence")]
    dependency_evidence: Vec<String>,
    #[arg(long = "compliance-evidence")]
    compliance_evidence: Vec<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct RepoCiRunsArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: i64,
    #[arg(long)]
    plane: Option<String>,
    #[arg(long = "suite-id")]
    suite_id: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TestRunArgs {
    #[arg(long)]
    full: bool,
    #[arg(long)]
    variant: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, default_value = "nightly")]
    plane: String,
    #[arg(long = "target-line", default_value = "main")]
    target_line: String,
    #[arg(long, default_value = "manual_full_test")]
    trigger: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TestStatusArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, default_value = "nightly")]
    plane: String,
    #[arg(long = "suite-id", default_value = "full_repo")]
    suite_id: String,
    #[arg(long, default_value_t = 20)]
    limit: i64,
    #[arg(long)]
    json: bool,
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
    #[arg(long, help = "Show storage, hygiene, and policy detail in text output")]
    verbose: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct DiffArgs {
    #[arg(long)]
    json: bool,
    #[arg(long, conflicts_with = "name_only")]
    stat: bool,
    #[arg(long = "name-only", conflicts_with = "stat")]
    name_only: bool,
    #[arg(long = "path")]
    paths: Vec<String>,
    #[arg(value_name = "PATH")]
    trailing_paths: Vec<String>,
    #[arg(long, default_value_t = DEFAULT_SNAPSHOT_DIFF_MAX_BYTES)]
    max_bytes: usize,
}

#[derive(Args, Clone)]
struct PullArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(
        long,
        help = "Import one remote line and fast-forward its local head when safe."
    )]
    line: Option<String>,
    #[arg(
        long,
        help = "Merge an explicitly divergent imported remote head into the current local line."
    )]
    merge: bool,
    #[arg(
        long,
        help = "Materialize the pulled line into the current workspace after the remote snapshot chain is imported."
    )]
    restore: bool,
    #[arg(
        long,
        conflicts_with = "merge",
        help = "Allow --restore to overwrite local workspace changes."
    )]
    force: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PushArgs {
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    line: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorMemoryRootArgs {
    #[arg(
        long,
        help = "Provision a missing supported RAM root and create its runtime directory."
    )]
    ensure: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorRuntimeRootArgs {
    #[arg(long = "server-data")]
    server_data: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorPostgresArgs {
    #[arg(long = "server-data")]
    server_data: Option<PathBuf>,
    #[arg(long, default_value = "postgres")]
    backend: String,
    #[arg(long)]
    dsn: Option<String>,
    #[arg(long = "content-schema")]
    content_schema: Option<String>,
    #[arg(long = "control-schema")]
    control_schema: Option<String>,
    #[arg(long)]
    connect: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorPlanAuthorityArgs {
    #[arg(long)]
    backend: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct DoctorPlanAuthorityWheelArgs {
    #[arg(long)]
    wheel: Option<PathBuf>,
    #[arg(long = "repack-installed")]
    repack_installed: bool,
    #[arg(long)]
    smoke: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct BlameArgs {
    path: String,
    #[arg(long, help = "Return blame for one line only.")]
    line: Option<usize>,
    #[arg(long = "start", help = "Start line for a bounded blame range.")]
    start_line: Option<usize>,
    #[arg(long = "end", help = "End line for a bounded blame range.")]
    end_line: Option<usize>,
    #[arg(
        long,
        help = "Restore only the selected line or range back into the current workspace file."
    )]
    restore: bool,
    #[arg(
        long = "dry-run",
        help = "Preview the scoped restore without writing the workspace file."
    )]
    dry_run: bool,
    #[arg(
        long = "snapshot",
        help = "Blame against one explicit immutable snapshot."
    )]
    snapshot_id: Option<String>,
    #[arg(
        long = "parent",
        help = "For a merge Snapshot, follow this direct parent instead of the primary parent."
    )]
    parent_snapshot_id: Option<String>,
    #[arg(
        long = "patchset",
        help = "Resolve one published patchset to its revision snapshot before blaming."
    )]
    patchset_id: Option<String>,
    #[arg(long = "remote", help = "Remote to use when resolving --patchset.")]
    remote_name: Option<String>,
    #[arg(
        long = "repo",
        help = "Resolve repo-scoped patchset refs within this remote repository."
    )]
    repo_name: Option<String>,
    #[arg(
        long = "change",
        help = "Required with repo-scoped numeric patchset refs."
    )]
    change_ref: Option<String>,
    #[arg(
        long = "plan-id",
        help = "Select one current Markdown lineage plan explicitly when the same artifact path is tracked by multiple current plans."
    )]
    plan_id: Option<String>,
    #[arg(
        long = "plan-ref",
        help = "Select one current Markdown lineage plan by artifact selector/ref."
    )]
    plan_ref: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ConfigShowArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
#[command(
    about = "Set the primary workflow-mode preset or advanced local overrides.",
    long_about = None
)]
struct ConfigSetArgs {
    #[arg(
        long = "repository-index",
        help = "Set the exact unsigned Binary Repository registry index used for remote authority routing."
    )]
    repository_index: Option<u32>,
    #[arg(
        long = "clear-repository-index",
        help = "Remove the stored Binary Repository registry index; remote authority operations then fail closed."
    )]
    clear_repository_index: bool,
    #[arg(
        long = "default-author-mode",
        help = "Set the default provenance author mode"
    )]
    default_author_mode: Option<String>,
    #[arg(
        long = "clear-default-author-mode",
        help = "Remove the stored default author mode"
    )]
    clear_default_author_mode: bool,
    #[arg(long = "default-model", help = "Set the default provenance model name")]
    default_model: Option<String>,
    #[arg(long = "clear-default-model", help = "Remove the stored default model")]
    clear_default_model: bool,
    #[arg(
        long = "task-tracking",
        help = "Set task workflow tracking: on or off."
    )]
    task_tracking: Option<String>,
    #[arg(
        long = "task-review",
        help = "Set task/outcome review requirement: on or off. Off lets `task land` auto-record `task_approve` using configured `user_name` when available."
    )]
    task_review: Option<String>,
    #[arg(
        long = "command-profiling",
        help = "Set command profiling artifact capture: on or off."
    )]
    command_profiling: Option<String>,
    #[arg(
        long = "task-worktree-alias-root",
        help = "Set the managed alias root for ephemeral task worktrees."
    )]
    task_worktree_alias_root: Option<String>,
    #[arg(
        long = "clear-task-worktree-alias-root",
        help = "Remove the stored managed alias-root override for task worktrees."
    )]
    clear_task_worktree_alias_root: bool,
    #[arg(
        long = "task-worktree-main-seed-ram-max-bytes",
        help = "Set the repo-local RAM budget, in bytes, for main-seed-backed task worktree bootstrap."
    )]
    task_worktree_main_seed_ram_max_bytes: Option<i64>,
    #[arg(
        long = "clear-task-worktree-main-seed-ram-max-bytes",
        help = "Remove the stored main-seed RAM budget override for task worktrees."
    )]
    clear_task_worktree_main_seed_ram_max_bytes: bool,
    #[arg(
        long = "task-auto-worktree",
        help = "Deprecated compatibility no-op; task-bound worktree bootstrap is always enabled.",
        hide = true
    )]
    legacy_task_auto_worktree: Option<String>,
    #[arg(
        long = "clear-task-auto-worktree",
        help = "Deprecated compatibility no-op; task-bound worktree bootstrap is always enabled.",
        hide = true
    )]
    legacy_clear_task_auto_worktree: bool,
    #[arg(
        long = "workflow-mode",
        help = "Primary workflow preset selector: solo_local, solo_remote, or team_remote."
    )]
    workflow_mode: Option<String>,
    #[arg(
        long = "workflow-default-scope",
        help = "Advanced override after workflow-mode presets for the default task/change workflow scope: local or remote."
    )]
    workflow_default_scope: Option<String>,
    #[arg(
        long = "clear-workflow-default-scope",
        help = "Remove the stored default task/change workflow scope."
    )]
    clear_workflow_default_scope: bool,
    #[arg(
        long = "task-default-scope",
        help = "Advanced override after workflow-mode presets for the default task command scope: local or remote."
    )]
    task_default_scope: Option<String>,
    #[arg(
        long = "clear-task-default-scope",
        help = "Remove the stored default task command scope."
    )]
    clear_task_default_scope: bool,
    #[arg(
        long = "change-default-scope",
        help = "Advanced override after workflow-mode presets for the default change command scope: local or remote."
    )]
    change_default_scope: Option<String>,
    #[arg(
        long = "clear-change-default-scope",
        help = "Remove the stored default change command scope."
    )]
    clear_change_default_scope: bool,
    #[arg(
        long = "id-namespace-prefix",
        help = "Set the optional namespace prefix used before workflow type codes such as T/C/P/PL/PR."
    )]
    id_namespace_prefix: Option<String>,
    #[arg(
        long = "clear-id-namespace-prefix",
        help = "Remove the stored namespace override and fall back to the default workflow namespace prefix."
    )]
    clear_id_namespace_prefix: bool,
    #[arg(
        long = "sprint",
        help = "Set sprint-style plan/task binding: on or off. On maps to required plan item refs; off disables the binding."
    )]
    sprint: Option<String>,
    #[arg(
        long = "plan-task-binding-mode",
        help = "Advanced override after workflow-mode presets for staged repo-local plan/task binding mode: off, advisory, strict, or required."
    )]
    plan_task_binding_mode: Option<String>,
    #[arg(
        long = "clear-plan-task-binding",
        help = "Remove stored plan/task binding overrides and fall back to staged defaults."
    )]
    clear_plan_task_binding: bool,
    #[arg(
        long = "user-name",
        help = "Set the default local user/display name for review actions"
    )]
    user_name: Option<String>,
    #[arg(
        long = "clear-user-name",
        help = "Remove the stored local user/display name"
    )]
    clear_user_name: bool,
    #[arg(
        long = "user-email",
        help = "Set the default local user email for review actions"
    )]
    user_email: Option<String>,
    #[arg(long = "clear-user-email", help = "Remove the stored local user email")]
    clear_user_email: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum PlanCommand {
    List(PlanListArgs),
    Show(ShowArgs),
    Revisions(PlanIdArgs),
    Items(ShowArgs),
    Candidates(CandidatesArgs),
    Inspect(ShowArgs),
    #[command(about = PLAN_SYNC_COMMAND_ABOUT)]
    Sync(SyncArgs),
}

#[derive(Subcommand)]
enum TaskCommand {
    #[command(
        about = "Start sprint work from one file-backed Plan item, or use an explicit title only when sprint mode is off; --from owns scoped Plan sync and taskability validation",
        override_usage = "ait-cli task start --intent <INTENT> (--from <MARKDOWN_PATH#ITEM_REF> [--title-override <TITLE>] | --title <TITLE>) [OPTIONS]"
    )]
    Start(TaskStartArgs),
    List(TaskListArgs),
    Show(TaskShowArgs),
    Tokens(TaskTokensArgs),
    Audit(TaskAuditArgs),
    #[command(
        about = TASK_LAND_COMMAND_ABOUT
    )]
    Land(TaskLandArgs),
    Canceled(TaskCanceledArgs),
    Restart(TaskRestartArgs),
    Publish(TaskPublishArgs),
}

#[derive(Subcommand)]
enum ChangeCommand {
    Create(ChangeCreateArgs),
    List(ChangeListArgs),
    Show(ChangeShowArgs),
    Revert(ChangeRevertArgs),
    Replay(ChangeReplayArgs),
    Close(ChangeCloseArgs),
    Publish(ChangePublishArgs),
}

#[derive(Subcommand)]
enum SnapshotCommand {
    Create(SnapshotCreateArgs),
    List(SnapshotListArgs),
    Show(SnapshotShowArgs),
    Diff(SnapshotDiffArgs),
    Revert(SnapshotRevertArgs),
    Replay(SnapshotReplayArgs),
    #[command(about = "Query bounded Snapshot ancestors or descendants using metadata-only DAG traversal.")]
    Ancestry(SnapshotAncestryArgs),
    #[command(
        name = "is-ancestor",
        about = "Test DAG ancestry; exits 0 when true, 1 when false, and 2 on lookup or storage errors."
    )]
    IsAncestor(SnapshotIsAncestorArgs),
    #[command(name = "merge-base", about = "Find the deterministic best common Snapshot ancestor.")]
    MergeBase(SnapshotMergeBaseArgs),
}

#[derive(Subcommand)]
enum StashCommand {
    #[command(about = "Save temporary local-only WIP without advancing the current line head.")]
    Save(StashSaveArgs),
    #[command(about = "List temporary local-only stashes without changing workspace content.")]
    List(JsonOnlyArgs),
    #[command(about = "Inspect one temporary stash without changing workspace content.")]
    Show(StashIdArgs),
    #[command(about = "Restore workspace content from a stash without dropping the stash record.")]
    Apply(StashRestoreArgs),
    #[command(about = "Restore workspace content from a stash and drop the stash record.")]
    Pop(StashRestoreArgs),
    #[command(about = "Drop a stash record without restoring workspace content.")]
    Drop(StashIdArgs),
}

#[derive(Subcommand)]
enum TagCommand {
    Create(TagCreateArgs),
    List(TagListArgs),
    Show(TagShowArgs),
    Delete(TagDeleteArgs),
}

#[derive(Subcommand)]
enum PatchsetCommand {
    Publish(PatchsetPublishArgs),
    List(PatchsetListArgs),
    Show(PatchsetShowArgs),
    Select(PatchsetSelectArgs),
    #[command(name = "ci-status")]
    CiStatus(PatchsetCiStatusArgs),
    #[command(name = "rerun-ci")]
    RerunCi(PatchsetRerunCiArgs),
}

#[derive(Subcommand)]
enum ReviewCommand {
    Show(ReviewShowArgs),
    Team {
        #[command(subcommand)]
        command: ReviewTeamCommand,
    },
    Task {
        #[command(subcommand)]
        command: ReviewTaskCommand,
    },
    Code {
        #[command(subcommand)]
        command: ReviewCodeCommand,
    },
    #[command(hide = true)]
    Request(ReviewRequestArgs),
    #[command(hide = true)]
    Approve(ReviewApproveArgs),
    #[command(name = "request-changes", hide = true)]
    RequestChanges(ReviewApproveArgs),
    #[command(hide = true)]
    Comment(ReviewApproveArgs),
    #[command(hide = true)]
    Defer(ReviewApproveArgs),
    #[command(name = "code-summary", hide = true)]
    CodeSummary(ReviewCodeSummaryArgs),
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
    Approve(ReviewApproveArgs),
    RequestChanges(ReviewApproveArgs),
    Comment(ReviewApproveArgs),
    Defer(ReviewApproveArgs),
}

#[derive(Subcommand)]
enum ReviewCodeCommand {
    Submit(ReviewCodeSubmitArgs),
    Template(ReviewCodeTemplateArgs),
}

#[derive(Subcommand)]
enum AttestCommand {
    Put(AttestPutArgs),
    Show(AttestShowArgs),
}

#[derive(Subcommand)]
enum PolicyCommand {
    Eval(PolicyEvalArgs),
    Show(PolicyShowArgs),
    Waive(PolicyWaiveArgs),
}

#[derive(Subcommand)]
enum WorktreeCommand {
    Status(WorktreeStatusArgs),
    Restore(WorktreeRestoreArgs),
    Show(WorktreeShowArgs),
    #[command(visible_alias = "open")]
    Path(WorktreePathArgs),
    Doctor(WorktreeDoctorArgs),
    #[command(name = "cleanup-candidates")]
    CleanupCandidates(WorktreeCleanupCandidatesArgs),
    Cleanup(WorktreeCleanupArgs),
    #[command(name = "prune-stale")]
    PruneStale(WorktreePruneStaleArgs),
    List(WorktreeListArgs),
    Sync(WorktreeSyncArgs),
    Recreate(WorktreeRecreateArgs),
    #[command(name = "recover-task")]
    RecoverTask(WorktreeRecoverTaskArgs),
    #[command(name = "restore-owned-head")]
    RestoreOwnedHead(WorktreeRestoreOwnedHeadArgs),
    Rebase(WorktreeRebaseArgs),
    Remove(WorktreeRemoveArgs),
}

#[derive(Args, Clone)]
struct QueryScopeArgs {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PlanListArgs {
    #[command(flatten)]
    scope: QueryScopeArgs,
    #[arg(long, help = "Show complete Plan history instead of the bounded active view")]
    all: bool,
}

#[derive(Args, Clone)]
struct PlanIdArgs {
    plan_id: String,
    #[command(flatten)]
    scope: QueryScopeArgs,
    #[arg(long, help = "Show complete revision history instead of the bounded newest view")]
    all: bool,
}

#[derive(Args, Clone)]
struct ShowArgs {
    plan_id: String,
    #[arg(long)]
    revision: Option<String>,
    #[command(flatten)]
    scope: QueryScopeArgs,
}

#[derive(Args, Clone)]
struct CandidatesArgs {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long = "all")]
    include_all: bool,
    #[arg(long)]
    contains: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SyncArgs {
    #[arg(help = "Markdown file or directory to reconcile into Plan revision lineage")]
    target: PathBuf,
    #[arg(long, help = "Select one structured [plan-ref: ...] root when the target is ambiguous")]
    plan_ref: Option<String>,
    #[arg(long, help = "Archive tracked Plan artifacts missing from the selected target")]
    prune: bool,
    #[arg(long, help = "Write local Plan lineage only; never create a Snapshot or advance a Line")]
    local: bool,
    #[arg(long, help = "Publish the touched local Plan heads to the named remote")]
    remote: Option<String>,
    #[arg(long, help = "Replay the current local Plan head after a divergent remote-head rejection")]
    rebase: bool,
    #[arg(
        long,
        help = "Safely adopt verified divergent or mixed local/remote Plan identity before publishing the reconciled head"
    )]
    reconcile: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum WorkflowCommand {
    #[command(
        about = "Show helper playbooks that collapse common inventory and landing command bursts."
    )]
    Guide(WorkflowGuideArgs),
    #[command(
        about = "Classify the current workspace as quick modification, normal Task, or fully governed and show required gates and escalation."
    )]
    Tier(WorkflowTierArgs),
    #[command(
        about = "Inventory cross-object Task, Change, Line, worktree, land, and Plan-binding state; dry-run is the default and never mutates Plan state."
    )]
    Reconcile(WorkflowReconcileArgs),
    #[command(
        about = "Show or apply the text-only ready-phase helper for one change before review and remote land."
    )]
    Ready(WorkflowReadyArgs),
    #[command(
        name = "land-local",
        about = "Run the local-only landing helper for one change onto a local target line."
    )]
    LandLocal(WorkflowLandLocalArgs),
    #[command(
        about = "Show or apply the review-and-land helper view for one change using workflow-mode scope defaults, routing shared closeout through task land with auto-rebase, target-line sync, and bound worktree cleanup."
    )]
    Land(WorkflowLandArgs),
}

#[derive(Args, Clone)]
struct TaskStartArgs {
    #[arg(
        long,
        help = "Unbound Task title; required and available only when sprint mode is off, and forbidden with --from",
        required_unless_present = "source",
        conflicts_with = "title_override"
    )]
    title: Option<String>,
    #[arg(long, help = "Required Task intent in both manual and --from modes")]
    intent: String,
    #[arg(
        long = "from",
        value_name = "MARKDOWN_PATH#ITEM_REF",
        help = "Sprint-only exact file-backed Plan source; syncs, validates, binds, and derives Task title deterministically",
        conflicts_with_all = ["title", "task_only"]
    )]
    source: Option<String>,
    #[arg(
        long = "title-override",
        help = "Exceptional explicit title for --from when the Plan item summary is unsuitable",
        requires = "source",
        conflicts_with = "title"
    )]
    title_override: Option<String>,
    #[arg(
        long = "task-only",
        help = "Create no initial Change; available only with the sprint-off manual title form"
    )]
    task_only: bool,
    #[arg(long = "change-title")]
    change_title: Option<String>,
    #[arg(long = "base-line")]
    base_line: Option<String>,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, help = "Show every Task-start progress phase")]
    verbose: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskListArgs {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, help = "Show complete Task history instead of the bounded open view")]
    all: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskShowArgs {
    task_id: String,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskTokensArgs {
    task_id: String,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long, value_parser = ["change", "worktree", "model"])]
    by: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskAuditArgs {
    task_id: String,
    #[arg(long = "target-line", default_value = "main")]
    target_line: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskLandArgs {
    #[arg(
        long = "all-completed-local",
        hide = true
    )]
    all_completed_local: bool,
    #[arg(
        help = "Task id or change id to land. Scope follows workflow_mode/task_default_scope unless --local or --remote is provided. Clean task worktrees are auto-rebased onto the target line before remote publish."
    )]
    task_or_change_id: Option<String>,
    #[arg(long = "snapshot-message")]
    snapshot_message: Option<String>,
    #[arg(long)]
    summary: Option<String>,
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
    reviewer: Option<String>,
    #[arg(long = "review-message")]
    review_message: Option<String>,
    #[arg(long)]
    target: Option<String>,
    #[arg(long, default_value = "direct")]
    mode: String,
    #[arg(
        long,
        help = "Force local draft land even when workflow_mode defaults to remote."
    )]
    local: bool,
    #[arg(
        long,
        help = "Force shared remote closeout or completed-local promotion using the named remote."
    )]
    remote: Option<String>,
    #[arg(
        long = "preview",
        help = "Show the task land state without mutating remote state or removing a worktree."
    )]
    preview: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskCanceledArgs {
    task_id: String,
    #[arg(long)]
    abandoned: bool,
    #[arg(long = "exclude-later-promotion")]
    exclude_later_promotion: bool,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskRestartArgs {
    task_id: String,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TaskPublishArgs {
    task_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeCreateArgs {
    task_id: String,
    #[arg(long)]
    title: String,
    #[arg(long = "base-line")]
    base_line: String,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeListArgs {
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, help = "Show complete Change history instead of the bounded open view")]
    all: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeShowArgs {
    change_id: String,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeRevertArgs {
    change_id: String,
    #[arg(long)]
    force: bool,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeReplayArgs {
    change_id: String,
    #[arg(long)]
    onto: String,
    #[arg(long)]
    force: bool,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangeCloseArgs {
    change_id: String,
    #[arg(long)]
    local: bool,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ChangePublishArgs {
    change_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum SnapshotProfile {
    Quick,
}

#[derive(Args, Clone)]
struct SnapshotCreateArgs {
    #[arg(long)]
    message: Option<String>,
    #[arg(
        long,
        value_enum,
        requires_all = ["message", "intent", "validation"]
    )]
    profile: Option<SnapshotProfile>,
    #[arg(long, requires = "profile")]
    intent: Option<String>,
    #[arg(long, requires = "profile")]
    validation: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotListArgs {
    #[arg(long, help = "Show complete Snapshot history instead of the bounded recent view")]
    all: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotShowArgs {
    snapshot_id: String,
    #[arg(long, help = "Show the complete Snapshot tree inventory")]
    files: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotDiffArgs {
    old_snapshot_id: String,
    new_snapshot_id: String,
    #[arg(long)]
    include_text: bool,
    #[arg(long, default_value_t = DEFAULT_SNAPSHOT_DIFF_MAX_BYTES)]
    max_bytes: usize,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotRevertArgs {
    snapshot_id: String,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotReplayArgs {
    snapshot_id: String,
    #[arg(long)]
    onto: String,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotAncestryArgs {
    snapshot_id: String,
    #[arg(long, conflicts_with = "descendants")]
    ancestors: bool,
    #[arg(long, conflicts_with = "ancestors")]
    descendants: bool,
    #[arg(long = "first-parent")]
    first_parent: bool,
    #[arg(long = "max-depth", default_value_t = DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_MAX_DEPTH)]
    max_depth: usize,
    #[arg(long, default_value_t = DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_LIMIT)]
    limit: usize,
    #[arg(long, help = "Show every result admitted by --limit instead of nearest evidence")]
    all: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotIsAncestorArgs {
    older_snapshot_id: String,
    newer_snapshot_id: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct SnapshotMergeBaseArgs {
    left_snapshot_id: String,
    right_snapshot_id: String,
    #[arg(long)]
    all: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct StashSaveArgs {
    #[arg(long)]
    message: Option<String>,
    #[arg(
        long = "keep-workspace",
        help = "Keep current workspace content after saving instead of restoring the current line head."
    )]
    keep_workspace: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct StashIdArgs {
    stash_id: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct StashRestoreArgs {
    stash_id: String,
    #[arg(
        long,
        help = "Overwrite unsaved workspace changes while restoring the stash."
    )]
    force: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TagCreateArgs {
    name: String,
    #[arg(long)]
    snapshot: Option<String>,
    #[arg(long)]
    message: String,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TagListArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TagShowArgs {
    name: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct TagDeleteArgs {
    name: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetPublishArgs {
    #[arg(long)]
    change: String,
    #[arg(long)]
    summary: String,
    #[arg(long = "author-mode")]
    author_mode: Option<String>,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetListArgs {
    #[arg(long)]
    change: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetShowArgs {
    patchset_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    change: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetSelectArgs {
    patchset_id: String,
    #[arg(long)]
    change: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetCiStatusArgs {
    patchset_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long = "recent-limit", default_value_t = 10)]
    recent_limit: i64,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct PatchsetRerunCiArgs {
    patchset_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long, default_value = "manual_rerun")]
    trigger: String,
    #[arg(long)]
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
    change_id: String,
    #[arg(long, default_value = "pass")]
    verdict: String,
    #[arg(long)]
    reviewer: Option<String>,
    #[arg(long = "patchset")]
    patchset_id: Option<String>,
    #[arg(long)]
    message: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewCodeSummaryArgs {
    change_id: String,
    #[arg(long)]
    reviewer: Option<String>,
    #[arg(long = "patchset")]
    patchset_id: Option<String>,
    #[arg(long)]
    message: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewCodeTemplateArgs {
    #[arg(long, default_value = "numbered")]
    style: String,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct ReviewShowArgs {
    change_id: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long)]
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
    name: Option<String>,
    #[arg(long = "snapshot")]
    snapshot_id: Option<String>,
    #[arg(long = "line")]
    line_name: Option<String>,
    #[arg(long, help = "Show baseline and workspace-root detail in text output")]
    verbose: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRestoreArgs {
    name: Option<String>,
    #[arg(long = "snapshot")]
    snapshot_id: Option<String>,
    #[arg(long = "line")]
    line_name: Option<String>,
    #[arg(long = "path")]
    paths: Vec<String>,
    #[arg(long)]
    force: bool,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeShowArgs {
    name: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreePathArgs {
    name: Option<String>,
    #[arg(long = "shell")]
    shell_output: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeDoctorArgs {
    #[arg(
        long,
        help = "Refresh each worktree's content status before building the doctor report."
    )]
    refresh: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeCleanupCandidatesArgs {
    #[arg(long = "older-than", default_value = "7d")]
    older_than: String,
    #[arg(long = "policy")]
    cleanup_policy: Option<String>,
    #[arg(long = "allow-manual-only")]
    allow_manual_only: bool,
    #[arg(long = "include-protected")]
    include_protected: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeCleanupArgs {
    #[arg(long = "older-than", default_value = "7d")]
    older_than: String,
    #[arg(long = "policy")]
    cleanup_policy: Option<String>,
    #[arg(long = "allow-manual-only")]
    allow_manual_only: bool,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    yes: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreePruneStaleArgs {
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeListArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    refresh: bool,
}

#[derive(Args, Clone)]
struct WorktreeSyncArgs {
    name: Option<String>,
    #[arg(long = "all")]
    all_worktrees: bool,
    #[arg(long = "line")]
    line_name: Option<String>,
    #[arg(long)]
    force: bool,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRecreateArgs {
    name: Option<String>,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRecoverTaskArgs {
    task_id: String,
    #[arg(long)]
    change: String,
    #[arg(long)]
    remote: Option<String>,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRestoreOwnedHeadArgs {
    name: Option<String>,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRebaseArgs {
    name: Option<String>,
    #[arg(long = "onto")]
    onto_line: Option<String>,
    #[arg(long = "continue")]
    continue_rebase: bool,
    #[arg(long = "abort")]
    abort_rebase: bool,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorktreeRemoveArgs {
    names: Vec<String>,
    #[arg(long = "all-stale")]
    all_stale: bool,
    #[arg(long = "delete-path")]
    delete_path: bool,
    #[arg(long)]
    force: bool,
    #[arg(long = "dry-run")]
    dry_run: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone)]
struct WorkflowGuideArgs {
    topic: Option<String>,
}

#[derive(Args, Clone)]
struct WorkflowTierArgs {
    #[arg(long, help = "Show static tier limits, gates, and ceremony comparison")]
    verbose: bool,
    #[arg(long)]
    json: bool,
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
    #[arg(long = "snapshot-message")]
    snapshot_message: Option<String>,
    #[arg(long)]
    summary: Option<String>,
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
}

#[derive(Args, Clone)]
struct WorkflowLandLocalArgs {
    change_id: String,
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    snapshot: Option<String>,
    #[arg(long = "snapshot-message")]
    snapshot_message: Option<String>,
}

#[derive(Args, Clone)]
struct WorkflowLandArgs {
    #[arg(
        help = "Change id to inspect or land. Scope follows workflow_mode unless --local or --remote is provided."
    )]
    change_id: Option<String>,
    #[arg(
        long = "all-completed-local",
        hide = true
    )]
    all_completed_local: bool,
    #[arg(
        long,
        help = "Apply the safe next land/closeout actions instead of only showing state."
    )]
    apply: bool,
    #[arg(long = "snapshot-message")]
    snapshot_message: Option<String>,
    #[arg(long)]
    summary: Option<String>,
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
    reviewer: Option<String>,
    #[arg(long = "review-message")]
    review_message: Option<String>,
    #[arg(long)]
    target: Option<String>,
    #[arg(long, default_value = "direct")]
    mode: String,
    #[arg(
        long,
        help = "Force local draft land even when workflow_mode defaults to remote."
    )]
    local: bool,
    #[arg(long, help = "Force shared remote closeout using the named remote.")]
    remote: Option<String>,
}
