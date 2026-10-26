import type { Buffer } from "node:buffer";

export type AgentTransport = "telegram" | "discord" | "slack" | "line";
export type AgentWorkerOperation =
  | "slack-command"
  | "discord-interaction"
  | "reply-provider";
export type JsonPrimitive = string | number | boolean | null;
export type JsonValue =
  | JsonPrimitive
  | JsonValue[]
  | { [key: string]: JsonValue };
/** Native JSON data keeps full-width integers as bigint. Use encodeJson for text. */
export type NativeJsonValue =
  | JsonPrimitive
  | bigint
  | NativeJsonValue[]
  | { [key: string]: NativeJsonValue };

export type JsonObject = { [key: string]: JsonValue };
export type EnvironmentOverrides = Record<
  string,
  string | number | boolean | null | undefined
>;

export const LANGUAGE_BINDING_CONTRACT: "ait.language.binding.v1";
/** @deprecated Chat integrations are retired in AIT 1.2.0. Legacy identifier only. */
export const AGENT_CAPABILITIES_CONTRACT: "ait.agent.worker.capabilities.v1";
/** @deprecated Chat integrations are retired in AIT 1.2.0. Legacy identifiers only. */
export const SUPPORTED_TRANSPORTS: readonly AgentTransport[];
/** @deprecated Chat integrations are retired in AIT 1.2.0. Legacy identifiers only. */
export const SUPPORTED_WORKER_OPERATIONS: readonly AgentWorkerOperation[];

export class NativeBridgeError extends Error {}
export class NativeResolutionError extends NativeBridgeError {}
export class NativeProtocolError extends NativeBridgeError {}

/** Native scope selection. Omitted/undefined fields retain Rust defaults. */
export interface ScopeOptions {
  local?: boolean;
  remoteName?: string;
}

export interface TaskListOptions extends ScopeOptions {
  all?: boolean;
}

export interface InitOptions {
  name?: string;
  defaultLine?: string;
  policyProfile?: string;
  defaultAuthorMode?: string;
  defaultModel?: string;
  repairExisting?: boolean;
}

export interface TaskStartRequest extends ScopeOptions {
  intent: string;
  title?: string;
  source?: string;
  editRoot?: string;
  /** Emit native progress directly to terminal stdout. Defaults to false. */
  humanProgress?: boolean;
}

export interface TaskQuickRequest extends ScopeOptions {
  intent: string;
  title?: string;
  editRoot?: string;
  /** Emit native progress directly to terminal stdout. Defaults to false. */
  humanProgress?: boolean;
}

export interface TaskFinishOptions extends ScopeOptions {
  message?: string;
  syncPlanMarkdown?: boolean;
}

export interface SnapshotCaptureOptions {
  message?: string;
  resumeSnapshotId?: string;
}

export interface PlanQueryOptions extends ScopeOptions {
  revision?: string;
}

export interface PlanCandidatesOptions extends ScopeOptions {
  includeAll?: boolean;
  contains?: string;
}

export interface PlanSyncOptions extends ScopeOptions {
  planRef?: string;
  prune?: boolean;
  reconcile?: boolean;
}

export interface PlanListOutputOptions extends ScopeOptions {
  all?: boolean;
}

export interface WorkflowReadyOptions {
  apply?: boolean;
  remoteName?: string;
  expectedIdentityPath?: string;
  snapshotMessage?: string;
  summary?: string;
  tests?: string;
  lint?: string;
  security?: string;
  license?: string;
  authorMode?: string;
  model?: string;
  runCiOnce?: boolean;
}

export interface WorkflowFinishOptions {
  apply?: boolean;
  remoteName?: string;
  expectedIdentityPath?: string;
  reviewer?: string;
  reviewMessage?: string;
}

export interface WorkflowReconcileOptions {
  remoteName?: string;
  taskId?: string;
  apply?: boolean;
  safeOnly?: boolean;
  scheduled?: boolean;
  /** Checked native usize. Omission uses 100; domain limits remain native. */
  limit?: number | bigint;
}

export interface WorkflowOutputOptions {
  remoteName?: string;
  full?: boolean;
}

export interface WorkflowPhaseResult {
  payload: NativeJsonValue;
  exitCode: number;
}

export interface SnapshotAncestryOptions {
  descendants?: boolean;
  firstParent?: boolean;
  maxDepth?: number | bigint;
  limit?: number | bigint;
}

export interface TaskListOutputOptions extends ScopeOptions {
  all?: boolean;
}

export interface SnapshotInspection {
  payload: NativeJsonValue;
  delta: NativeJsonValue;
}

export interface LineListOptions {
  includeAll?: boolean;
  archived?: boolean;
  remoteName?: string;
}

export interface LineCreateOptions {
  fromSnapshot?: string;
  switch?: boolean;
}

export interface LineSwitchOptions {
  restore?: boolean;
  force?: boolean;
}

export interface LineDeleteOptions {
  remoteName?: string;
  confirm?: boolean;
}

export interface LineMergeOptions {
  source?: string;
  message?: string;
  continueMerge?: boolean;
  abortMerge?: boolean;
}

export interface LineCleanupOptions {
  idleFor?: string;
  cleanupKind?: string;
  limit?: number | bigint;
  includeProtected?: boolean;
  apply?: boolean;
}

export interface LineCleanupOutputOptions {
  all?: boolean;
  includeProtected?: boolean;
  cleanupKind?: string;
  limit?: number | bigint;
}

export interface LineInventory {
  payload: NativeJsonValue;
  current_line: string;
}

export interface RepositoryJobsOptions {
  remoteName?: string;
  workerJobIndex?: number | bigint;
  state?: string;
  limit?: number | bigint;
}

export interface RemoteOptions {
  remoteName?: string;
}

export interface RemoteRepositoryOptions extends RemoteOptions {
  repoName?: string;
}

export interface ChangeQueryOptions extends ScopeOptions {
  repoName?: string;
}

export interface ChangeMutationOptions extends ChangeQueryOptions {
  force?: boolean;
  dryRun?: boolean;
}

export interface PatchsetPublishOptions extends RemoteOptions {
  authorMode?: string;
}

export interface PatchsetPublishExplicitOptions extends PatchsetPublishOptions {
  repoName?: string;
}

export interface ReviewTeamApproveOptions extends RemoteOptions {
  patchsetId?: string;
  reviewer?: string;
  message?: string;
}

export interface ReviewRequestOptions extends RemoteOptions {
  patchsetId?: string;
  note?: string;
}

export interface ReviewRecordOptions extends ReviewTeamApproveOptions {
  blocking?: boolean;
}

export interface ReviewCodeSubmitOptions extends RemoteOptions {
  reviewer?: string;
}

export interface ReviewCodeTemplateOptions {
  style?: string;
}

export interface AttestationPutOptions extends RemoteRepositoryOptions {
  patchsetId?: string;
  changeId?: string;
  tests?: string;
  lint?: string;
  security?: string;
  license?: string;
  authorMode?: string;
  model?: string;
}

export interface PolicyWaiveOptions extends RemoteRepositoryOptions {
  expiresAt?: string;
}

export interface ChangeCreateOptions extends ScopeOptions {
  baseLine?: string;
  forkSnapshot?: string;
}

export interface ChangeOperationOptions extends ScopeOptions {
  force?: boolean;
  dryRun?: boolean;
}

export interface ChangeListOutputOptions extends ScopeOptions {
  all?: boolean;
}

export interface SnapshotMutationOptions {
  force?: boolean;
  dryRun?: boolean;
}

export interface WorkspaceRestoreOptions extends SnapshotMutationOptions {
  targetSnapshotId?: string;
  baselineSnapshotId?: string;
}

export interface SnapshotDiffOptions {
  includeText?: boolean;
  maxBytes?: number | bigint;
}

export interface SnapshotExplicitCaptureOptions {
  message?: string;
  parentSnapshotId?: string;
  updateLineRef?: boolean;
  touchLine?: boolean;
  recordWorkflowMetadata?: boolean;
}

export interface SnapshotRestoreLinesOptions {
  line?: number | bigint;
  startLine?: number | bigint;
  endLine?: number | bigint;
  yes?: boolean;
}

export interface NativeWorkspaceCommandLock {
  metadata(): NativeJsonValue;
  close(): void;
  release(): void;
  isClosed(): boolean;
}

export interface NativeWorkspaceCommandLockConstructor {
  new(repoRoot: string, commandName: string): NativeWorkspaceCommandLock;
}

export class WorkspaceCommandLock implements NativeWorkspaceCommandLock {
  constructor(repoRoot: string, commandName: string, runtime?: NativeRuntime);
  metadata(): NativeJsonValue;
  close(): void;
  release(): void;
  isClosed(): boolean;
}

export interface WorktreeNameOptions {
  name?: string;
}

export interface WorktreeRefreshOptions {
  refreshStatus?: boolean;
}

export interface WorktreeGetOptions extends WorktreeNameOptions, WorktreeRefreshOptions {}

export interface WorktreeStatusOptions extends WorktreeNameOptions {
  snapshotId?: string;
  lineName?: string;
}

export interface WorktreeMutationOptions {
  force?: boolean;
  dryRun?: boolean;
}

export interface WorktreeRestoreOptions extends WorktreeStatusOptions, WorktreeMutationOptions {
  paths?: string[];
}

export interface WorktreeCleanupCandidatesOptions {
  olderThan?: string;
  cleanupPolicy?: string;
  includeProtected?: boolean;
  allowManualOnly?: boolean;
}

export interface WorktreeCleanupOptions {
  olderThan?: string;
  cleanupPolicy?: string;
  allowManualOnly?: boolean;
  limit?: number | bigint;
  dryRun?: boolean;
}

export interface WorktreeDryRunOptions {
  dryRun?: boolean;
}

export interface WorktreeRemoveOptions extends WorktreeMutationOptions {
  allStale?: boolean;
  deletePath?: boolean;
}

export interface WorktreeBindExistingOptions {
  taskId?: string;
  changeId?: string;
  autoCreatedForTask?: boolean;
  targetBaseLine?: string;
  forkSnapshotId?: string;
  forkedFromLine?: string;
}

export interface WorktreeSyncOptions extends WorktreeNameOptions, WorktreeMutationOptions {
  lineName?: string;
}

export interface WorktreeRecoveryOptions extends WorktreeNameOptions, WorktreeDryRunOptions {}

export interface WorktreeRebaseOptions extends WorktreeNameOptions {
  ontoLineName?: string;
}

export interface StashSaveOptions {
  message?: string;
  keepWorkspace?: boolean;
}

export interface ReviewTaskOptions extends RemoteOptions {
  patchsetId?: string;
  message?: string;
}

export interface ReviewTaskOperationResult {
  payload: NativeJsonValue;
  package_review: boolean;
}

export interface BlameOptions {
  line?: number | bigint;
  startLine?: number | bigint;
  endLine?: number | bigint;
  snapshotId?: string;
  viaParentSnapshotId?: string;
  patchsetId?: string;
  remoteName?: string;
  planId?: string;
  planRef?: string;
}

export interface RemoteLineOptions extends RemoteOptions {
  lineName?: string;
}

export interface PullOptions extends RemoteLineOptions {
  merge?: boolean;
  restore?: boolean;
  force?: boolean;
}

export interface SnapshotUploadOptions extends RemoteLineOptions {
  reason?: string;
}

export interface WorktreeCleanupCommandOptions extends WorktreeCleanupOptions {
  yes?: boolean;
}

export interface WorktreePruneCommandOptions extends WorktreeDryRunOptions {
  yes?: boolean;
}

export interface WorktreeSyncCommandOptions extends WorktreeSyncOptions {
  allWorktrees?: boolean;
}

export interface WorktreeRecoverTaskOptions extends RemoteOptions, WorktreeDryRunOptions {
  changeRef?: string;
}

export interface WorktreeRebaseCommandOptions extends WorktreeNameOptions, WorktreeDryRunOptions {
  ontoLine?: string;
  continueRebase?: boolean;
  abortRebase?: boolean;
}

export interface WorktreeRemoveCommandOptions extends WorktreeRemoveOptions {
  yes?: boolean;
}

export interface GitImportOptions {
  allBranchesAndTags?: boolean;
  dryRun?: boolean;
}

export interface GitExportOptions {
  allLinesAndTags?: boolean;
  dryRun?: boolean;
  generation?: string;
  gitObject?: string;
}

export interface EffectivePolicyOptions {
  contentClass?: string;
  authorClass?: string;
  fallbackProfile?: string;
}

export interface WorkflowIdOptions {
  namespacePrefix?: string;
  includeLegacy?: boolean;
}

export interface WorkflowNamespaceOptions extends WorkflowIdOptions {
  includeTaskChangeOrigins?: boolean;
}

export interface WorkflowSequenceOptions {
  namespacePrefix?: string;
  width?: number | bigint;
}

export type TaskStatusDetails = {
  normalized_status: string | null;
  display_label: string | null;
  closed: boolean;
};

export type WorkflowResultEnvelope = {
  ok: boolean;
  kind: string;
  value: NativeJsonValue;
  error: { code: string; message: string; detail: NativeJsonValue } | null;
};

export type ParsedPlanReference = {
  plan_ref: string;
  heading_title: string;
  heading_level: number | bigint;
  line_number: number | bigint;
};

export type ParsedPlanItem = {
  plan_item_ref: string;
  text: string;
  checkbox_state: string;
  heading_path: string[];
  line_number: number | bigint;
};

export type PlanRefsResult = {
  plan_ref_count: number | bigint;
  plan_refs: ParsedPlanReference[];
};

export type ParsedPlan = PlanRefsResult & {
  markdown_text: string;
  item_count: number | bigint;
  items: ParsedPlanItem[];
};

export type ParsedPlanReferenceInput = {
  plan_ref: string;
  heading_title?: string | null;
  heading_level?: number | bigint | null;
  line_number?: number | bigint | null;
};

export type ParsedPlanInput = {
  markdown_text?: string | null;
  plan_ref_count?: number | bigint | null;
  item_count?: number | bigint | null;
  plan_refs?: ParsedPlanReferenceInput[] | null;
  items?: PlanItemInput[] | null;
};

export interface ManifestIdentityOptions {
  oldSnapshotId?: string;
  newSnapshotId?: string;
}

export interface ManifestTextDiffOptions extends ManifestIdentityOptions {
  blobBytesById?: Record<string, Buffer>;
  includeText?: boolean;
  maxBytes?: number | bigint;
}

export interface CurrentSourceCacheRequest {
  namespaceRoot: string;
  coreRepoRoot: string;
  extSuffix: string;
  rustflags?: string;
  workerId?: string;
  coreSourceFingerprint?: string;
  serverSourceFingerprint?: string;
}

export interface CurrentSourceCacheActivateOptions {
  extraJson?: string;
  registerLease?: boolean;
  ownerPid?: number | bigint;
}

export interface CurrentSourceCachePruneOptions {
  now?: number;
  idleTtlSeconds?: number | bigint;
  buildStaleSeconds?: number | bigint;
  maxBytes?: number | bigint;
  removeUnleasedReady?: boolean;
}

export interface CurrentSourceExtensionFreshnessRequest {
  metadataPath: string;
  extensionPath: string;
  sourceMtimeNs: number | bigint;
  sourceFingerprint: string;
}

export interface CurrentSourceBinaryFreshnessRequest {
  metadataPath: string;
  binaryPath: string;
  metadataFingerprintKey: string;
  metadataSourceMtimeKey: string;
  metadataMtimeKey: string;
  metadataShaKey: string;
  sourceMtimeNs: number | bigint;
  sourceFingerprint: string;
}

export type PlanItemInput = {
  plan_item_ref: string;
  text?: string | null;
  checkbox_state?: string | null;
  heading_path?: string[] | null;
  line_number?: number | bigint | null;
};

export type ExtractedPlanSection = ParsedPlanReference & {
  section_markdown: string;
  items: ParsedPlanItem[];
};

export interface PlanDispatchOptions {
  tasks?: NativeJsonValue;
  revision?: NativeJsonValue;
  localShadow?: NativeJsonValue;
}

export interface PlanCandidatesPayloadOptions {
  scope?: string;
  repoName?: string;
  remote?: string;
  includeAll?: boolean;
}

export type PlanTaskLinkIndexes = {
  by_item: { plan_id: string; plan_item_ref: string; tasks: NativeJsonValue[] }[];
  by_plan: Record<string, NativeJsonValue[]>;
};

export interface VisibleWorkspaceOptions {
  ignoreRulesText?: string;
  runtimeRoot?: string;
}

export type ResolvedRepoArtifactPath = {
  resolved_path: string;
  artifact_path: string;
};

export interface BuildPackMembersOptions {
  maxDeltaChainDepth?: number | bigint;
  initialByPath?: NativeJsonValue;
}

export interface StorageValidationOptions {
  packedBlobCount: number | bigint;
  packedFullBlobCount: number | bigint;
  packedDeltaBlobCount: number | bigint;
  packCount: number | bigint;
  packIndexErrorCount: number | bigint;
  storageSavingsRatio: number;
  treePackIndexErrorCount?: number | bigint;
  unreferencedBlobCount?: number | bigint;
  unreferencedTreeCount?: number | bigint;
  signalsSummary?: NativeJsonValue;
}

export interface BuildSnapshotIdOptions {
  repoName: string;
  lineName: string;
  rootTreeId: string;
  parentSnapshotId?: string;
  message?: string;
  snapshotKind?: string;
}

export type PlanHttpRequest = {
  method: string;
  url: string;
  body: NativeJsonValue;
};

export interface CreatePlanInput {
  repoName: string;
  title: string;
  artifactPath: string;
  artifactHeading: string;
  items: NativeJsonValue[];
  artifactSelector?: string;
  summary?: string;
  status?: string;
  planId?: string;
  sourceKind?: string;
  artifactBody?: string;
}

export interface RevisePlanInput {
  planId: string;
  artifactPath: string;
  artifactHeading: string;
  items: NativeJsonValue[];
  artifactSelector?: string;
  title?: string;
  summary?: string;
  sourceKind?: string;
  artifactBody?: string;
  expectedHeadRevisionId?: string;
}

export interface PlanHttpOptions {
  headersJson?: string;
  timeoutMs?: number | bigint;
  retryAttempts?: number | bigint;
  retryBackoffMs?: number | bigint;
  poolMaxIdlePerHost?: number | bigint;
  repositoryIndex?: number | bigint;
}

export interface TaskPlanLinkageOptions {
  planId?: string;
  originPlanRevisionId?: string;
  planItemRef?: string;
}

export interface CreatePlanningSessionOptions {
  title?: string;
  mode?: string;
  preferredAgent?: string;
  resumeIfActive?: boolean;
  planningSessionId?: string;
}

export interface PlanningSessionEventsOptions {
  afterSequence?: number | bigint;
  limit?: number | bigint;
}

export interface JoinPlanningSessionOptions {
  surface?: string;
  title?: string;
  modelName?: string;
  resumeIfActive?: boolean;
}

export interface PromotePlanningSessionInput {
  artifactPath: string;
  artifactSelector: string;
  artifactHeading: string;
  items: NativeJsonValue[];
  title?: string;
  summary?: string;
  artifactBody?: string;
}

export const DEFAULT_REQUEST_TIMEOUT_SECONDS: 30;
export const LONG_RUNNING_REQUEST_TIMEOUT_SECONDS: 1800;

export type NativeHttpClientStats = {
  base_url: string;
  default_timeout_ms: number | bigint;
  retry_attempts: number | bigint;
  retry_backoff_ms: number | bigint;
  pool_max_idle_per_host: number | bigint;
  request_count: number | bigint;
  retry_count: number | bigint;
  closed: boolean;
};

export interface NativePlanRemoteTransportConfig {
  requestTimeoutSeconds(longRunning: boolean): number;
}

export interface NativePlanRemoteTransportConfigConstructor {
  new(requestTimeoutSeconds?: number | null, longRunningRequestTimeoutSeconds?: number | null): NativePlanRemoteTransportConfig;
}

export class PlanRemoteTransportConfig implements NativePlanRemoteTransportConfig {
  constructor(requestTimeoutSeconds?: number | null, longRunningRequestTimeoutSeconds?: number | null, runtime?: NativeRuntime);
  requestTimeoutSeconds(longRunning: boolean): number;
}

export interface NativePlanHttpClientManager {
  inspect(): NativeHttpClientStats;
  close(): NativeHttpClientStats;
  listPlans(repoName: string, artifactPath?: string | null): NativeJsonValue[];
  getPlan(planId: string): NativeJsonValue;
  listPlanRevisions(planId: string): NativeJsonValue[];
  getPlanRevision(planId: string, planRevisionId: string): NativeJsonValue;
  createPlan(input: CreatePlanInput): NativeJsonValue;
  revisePlan(input: RevisePlanInput): NativeJsonValue;
  updatePlanStatus(planId: string, status: string): NativeJsonValue;
  putPlanRevisionArtifacts(planId: string, planRevisionId: string, artifacts: NativeJsonValue[]): NativeJsonValue;
  createPlanningSession(planId: string, input?: CreatePlanningSessionOptions | null): NativeJsonValue;
  listPlanningSessions(planId: string, status?: string | null): NativeJsonValue[];
  getPlanningSession(planningSessionId: string): NativeJsonValue;
  appendPlanningSessionEvent(planningSessionId: string, eventType: string, payload?: NativeJsonValue | null): NativeJsonValue;
  listPlanningSessionEvents(planningSessionId: string, input?: PlanningSessionEventsOptions | null): NativeJsonValue[];
  joinPlanningSession(planningSessionId: string, input?: JoinPlanningSessionOptions | null): NativeJsonValue;
  promotePlanningSession(planningSessionId: string, input: PromotePlanningSessionInput): NativeJsonValue;
  closePlanningSession(planningSessionId: string, status?: string | null): NativeJsonValue;
}

export interface NativePlanHttpClientManagerConstructor {
  new(baseUrl: string, options?: PlanHttpOptions | null): NativePlanHttpClientManager;
}

export class PlanHttpClientManager implements NativePlanHttpClientManager {
  constructor(baseUrl: string, options?: PlanHttpOptions | null, runtime?: NativeRuntime);
  inspect(): NativeHttpClientStats;
  close(): NativeHttpClientStats;
  listPlans(repoName: string, artifactPath?: string | null): NativeJsonValue[];
  getPlan(planId: string): NativeJsonValue;
  listPlanRevisions(planId: string): NativeJsonValue[];
  getPlanRevision(planId: string, planRevisionId: string): NativeJsonValue;
  createPlan(input: CreatePlanInput): NativeJsonValue;
  revisePlan(input: RevisePlanInput): NativeJsonValue;
  updatePlanStatus(planId: string, status: string): NativeJsonValue;
  putPlanRevisionArtifacts(planId: string, planRevisionId: string, artifacts: NativeJsonValue[]): NativeJsonValue;
  createPlanningSession(planId: string, input?: CreatePlanningSessionOptions | null): NativeJsonValue;
  listPlanningSessions(planId: string, status?: string | null): NativeJsonValue[];
  getPlanningSession(planningSessionId: string): NativeJsonValue;
  appendPlanningSessionEvent(planningSessionId: string, eventType: string, payload?: NativeJsonValue | null): NativeJsonValue;
  listPlanningSessionEvents(planningSessionId: string, input?: PlanningSessionEventsOptions | null): NativeJsonValue[];
  joinPlanningSession(planningSessionId: string, input?: JoinPlanningSessionOptions | null): NativeJsonValue;
  promotePlanningSession(planningSessionId: string, input: PromotePlanningSessionInput): NativeJsonValue;
  closePlanningSession(planningSessionId: string, status?: string | null): NativeJsonValue;
}

export interface NativeTaskWorkflowHttpClientManager {
  inspect(): NativeHttpClientStats;
  close(): NativeHttpClientStats;
}

export interface NativeTaskWorkflowHttpClientManagerConstructor {
  new(baseUrl: string, options?: PlanHttpOptions | null): NativeTaskWorkflowHttpClientManager;
}

export class TaskWorkflowHttpClientManager implements NativeTaskWorkflowHttpClientManager {
  constructor(baseUrl: string, options?: PlanHttpOptions | null, runtime?: NativeRuntime);
  inspect(): NativeHttpClientStats;
  close(): NativeHttpClientStats;
}

export interface CreateRemoteTaskOptions {
  taskId?: string;
  planId?: string;
  originPlanRevisionId?: string;
  planItemRef?: string;
}

export interface CreateRemoteChangeOptions {
  changeId?: string;
  forkSnapshotId?: string;
  forkedFromLine?: string;
}

export interface RemoteLineHeadOptions {
  headSnapshotId?: string;
  expectedHeadSnapshotId?: string;
}

export interface RemoteSnapshotOptions {
  includeContent?: boolean;
  path?: string;
}

export interface RemoteTargetOptions {
  repoName?: string;
  exactId?: boolean;
}

export interface GetRemotePatchsetOptions {
  repoName?: string;
  changeRef?: string;
}

export interface PublishRemotePatchsetInput extends RemoteTargetOptions {
  changeId: string;
  baseSnapshotId: string;
  revisionSnapshotId: string;
  summary: string;
  authorMode: string;
}

export interface RemotePatchsetCiOptions extends RemoteTargetOptions {
  trigger?: string;
  executionProfile?: string;
}

export interface RemotePatchsetCiStatusOptions extends RemoteTargetOptions {
  recentLimit?: number | bigint;
}

export interface RemoteReviewRequestInput extends RemoteTargetOptions {
  changeId: string;
  patchsetId: string;
  reviewerGroups: string[];
  note?: string;
}

export interface RemoteReviewRecordInput extends RemoteTargetOptions {
  changeId: string;
  patchsetId: string;
  reviewer: string;
  action: string;
  comment?: string;
  blocking?: boolean;
}

export interface RemoteWaiverInput extends RemoteTargetOptions {
  patchsetId: string;
  ruleName: string;
  reason: string;
  expiresAt?: string;
}

export interface RemoteSubmissionInput {
  changeId: string;
  patchsetId?: string;
  targetLine?: string;
  mode?: string;
  repoName?: string;
}

export interface RemoteCloseTaskOptions {
  status?: string;
  repoName?: string;
}

export interface RemoteMutationReceiptInput {
  action: string;
  sourceAction: string;
  delivery: string;
  responseRecovery?: NativeJsonValue;
  result?: NativeJsonValue;
}

export interface NativeHttpTaskRemote {
  inspectClient(): NativeHttpClientStats;
  closeClient(): NativeHttpClientStats;
  changeLineagePayload(baseLine: string, lineRow?: NativeJsonValue | null): NativeJsonValue;
  getLine(repoName: string, lineName: string): NativeJsonValue;
  getTask(taskId: string, repoName?: string | null): NativeJsonValue;
  listTasks(repoName: string): NativeJsonValue[];
  createTask(repoName: string, title: string, intent: string, options?: CreateRemoteTaskOptions | null): NativeJsonValue;
  listChanges(repoName: string): NativeJsonValue[];
  getChangeDetail(changeRef: string, repoName?: string | null): NativeJsonValue;
  getChange(changeRef: string, repoName?: string | null): NativeJsonValue;
  closeChange(changeRef: string, status?: string | null, repoName?: string | null): NativeJsonValue;
  createChange(repoName: string, taskId: string, title: string, baseLine: string, options?: CreateRemoteChangeOptions | null): NativeJsonValue;
  updateRemoteLine(repoName: string, lineName: string, options?: RemoteLineHeadOptions | null): NativeJsonValue;
  planRemoteZstdBulk(repoName: string, request: NativeJsonValue): NativeJsonValue;
  putRemoteZstdObjectPack(repoName: string, packId: string, packBytes: Buffer): NativeJsonValue;
  putRemoteZstdTreePack(repoName: string, packId: string, packBytes: Buffer): NativeJsonValue;
  commitRemoteZstdBulk(repoName: string, request: NativeJsonValue): NativeJsonValue;
  getRemoteZstdImportManifest(repoName: string, snapshotId: string): NativeJsonValue;
  getRemoteZstdObjectPack(repoName: string, packId: string): Buffer;
  getRemoteZstdTreePack(repoName: string, packId: string): Buffer;
  getRemoteSnapshot(repoName: string, snapshotId: string, options?: RemoteSnapshotOptions | null): NativeJsonValue;
  getRemoteSnapshotsExistence(repoName: string, snapshotIds: string[]): NativeJsonValue;
}

export interface NativeHttpTaskRemoteConstructor {
  new(baseUrl: string, options?: PlanHttpOptions | null): NativeHttpTaskRemote;
}

export class HttpTaskRemote implements NativeHttpTaskRemote {
  constructor(baseUrl: string, options?: PlanHttpOptions | null, runtime?: NativeRuntime);
  inspectClient(): NativeHttpClientStats;
  closeClient(): NativeHttpClientStats;
  changeLineagePayload(baseLine: string, lineRow?: NativeJsonValue | null): NativeJsonValue;
  getLine(repoName: string, lineName: string): NativeJsonValue;
  getTask(taskId: string, repoName?: string | null): NativeJsonValue;
  listTasks(repoName: string): NativeJsonValue[];
  createTask(repoName: string, title: string, intent: string, options?: CreateRemoteTaskOptions | null): NativeJsonValue;
  listChanges(repoName: string): NativeJsonValue[];
  getChangeDetail(changeRef: string, repoName?: string | null): NativeJsonValue;
  getChange(changeRef: string, repoName?: string | null): NativeJsonValue;
  closeChange(changeRef: string, status?: string | null, repoName?: string | null): NativeJsonValue;
  createChange(repoName: string, taskId: string, title: string, baseLine: string, options?: CreateRemoteChangeOptions | null): NativeJsonValue;
  updateRemoteLine(repoName: string, lineName: string, options?: RemoteLineHeadOptions | null): NativeJsonValue;
  planRemoteZstdBulk(repoName: string, request: NativeJsonValue): NativeJsonValue;
  putRemoteZstdObjectPack(repoName: string, packId: string, packBytes: Buffer): NativeJsonValue;
  putRemoteZstdTreePack(repoName: string, packId: string, packBytes: Buffer): NativeJsonValue;
  commitRemoteZstdBulk(repoName: string, request: NativeJsonValue): NativeJsonValue;
  getRemoteZstdImportManifest(repoName: string, snapshotId: string): NativeJsonValue;
  getRemoteZstdObjectPack(repoName: string, packId: string): Buffer;
  getRemoteZstdTreePack(repoName: string, packId: string): Buffer;
  getRemoteSnapshot(repoName: string, snapshotId: string, options?: RemoteSnapshotOptions | null): NativeJsonValue;
  getRemoteSnapshotsExistence(repoName: string, snapshotIds: string[]): NativeJsonValue;
}

export interface NativeHttpWorkflowCloseoutRemote {
  inspectClient(): NativeHttpClientStats;
  closeClient(): NativeHttpClientStats;
  mutationReceipt(input: RemoteMutationReceiptInput): NativeJsonValue;
  actionMutationReceipts(code: string, result: NativeJsonValue): NativeJsonValue;
  listPatchsets(changeId: string, repoName?: string | null): NativeJsonValue[];
  getPatchset(patchsetId: string, options?: GetRemotePatchsetOptions | null): NativeJsonValue;
  publishPatchset(input: PublishRemotePatchsetInput): NativeJsonValue;
  selectPatchset(changeId: string, patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  runPatchsetCi(patchsetId: string, options?: RemotePatchsetCiOptions | null): NativeJsonValue;
  readPatchsetCiStatus(patchsetId: string, options?: RemotePatchsetCiStatusOptions | null): NativeJsonValue;
  requestReview(input: RemoteReviewRequestInput): NativeJsonValue;
  listReviews(changeId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  recordReview(input: RemoteReviewRecordInput): NativeJsonValue;
  putAttestation(patchsetId: string, authorMode: string, statement: NativeJsonValue, options?: RemoteTargetOptions | null): NativeJsonValue;
  getAttestation(patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  evaluatePolicy(patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  getPolicy(patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  createWaiver(input: RemoteWaiverInput): NativeJsonValue;
  submitLand(input: RemoteSubmissionInput): NativeJsonValue;
  getLand(submissionId: string, repoName?: string | null): NativeJsonValue;
  retryLand(submissionId: string, reason?: string | null, repoName?: string | null): NativeJsonValue;
  closeTask(taskId: string, options?: RemoteCloseTaskOptions | null): NativeJsonValue;
}

export interface NativeHttpWorkflowCloseoutRemoteConstructor {
  new(baseUrl: string, options?: PlanHttpOptions | null): NativeHttpWorkflowCloseoutRemote;
}

export class HttpWorkflowCloseoutRemote implements NativeHttpWorkflowCloseoutRemote {
  constructor(baseUrl: string, options?: PlanHttpOptions | null, runtime?: NativeRuntime);
  inspectClient(): NativeHttpClientStats;
  closeClient(): NativeHttpClientStats;
  mutationReceipt(input: RemoteMutationReceiptInput): NativeJsonValue;
  actionMutationReceipts(code: string, result: NativeJsonValue): NativeJsonValue;
  listPatchsets(changeId: string, repoName?: string | null): NativeJsonValue[];
  getPatchset(patchsetId: string, options?: GetRemotePatchsetOptions | null): NativeJsonValue;
  publishPatchset(input: PublishRemotePatchsetInput): NativeJsonValue;
  selectPatchset(changeId: string, patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  runPatchsetCi(patchsetId: string, options?: RemotePatchsetCiOptions | null): NativeJsonValue;
  readPatchsetCiStatus(patchsetId: string, options?: RemotePatchsetCiStatusOptions | null): NativeJsonValue;
  requestReview(input: RemoteReviewRequestInput): NativeJsonValue;
  listReviews(changeId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  recordReview(input: RemoteReviewRecordInput): NativeJsonValue;
  putAttestation(patchsetId: string, authorMode: string, statement: NativeJsonValue, options?: RemoteTargetOptions | null): NativeJsonValue;
  getAttestation(patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  evaluatePolicy(patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  getPolicy(patchsetId: string, options?: RemoteTargetOptions | null): NativeJsonValue;
  createWaiver(input: RemoteWaiverInput): NativeJsonValue;
  submitLand(input: RemoteSubmissionInput): NativeJsonValue;
  getLand(submissionId: string, repoName?: string | null): NativeJsonValue;
  retryLand(submissionId: string, reason?: string | null, repoName?: string | null): NativeJsonValue;
  closeTask(taskId: string, options?: RemoteCloseTaskOptions | null): NativeJsonValue;
}

export interface UsageBundleOptions {
  usageRoles?: string[];
  usageScope?: string;
}

export interface TransportBindingMetadataInput {
  transport: string;
  surfaceId: string;
  conversationKey: string;
  surfaceTitle?: string;
  surfaceKind?: string;
  threadId?: string;
  replyTarget?: NativeJsonValue;
  metadataExtra?: NativeJsonValue;
}

export interface TransportEventInput {
  transport: string;
  actorIdentity: string;
  channelId: string;
  text: string;
  actorTransportId?: string;
  actorUsername?: string;
  actorDisplayName?: string;
  actorIsBot?: boolean;
  channelTitle?: string;
  channelKind?: string;
  threadId?: string;
  occurredAt?: string;
  eventId?: string;
  dedupeKey?: string;
  messageId?: NativeJsonValue;
  messageIds?: NativeJsonValue;
  attachments?: NativeJsonValue;
  metadata?: NativeJsonValue;
}

export interface TransportReplyInput {
  transport: string;
  channelId: string;
  text: string;
  channelTitle?: string;
  channelKind?: string;
  threadId?: string;
  replyToEventId?: string;
  deliveryKind?: string;
  replyToMessageId?: NativeJsonValue;
  replyToMessageIds?: NativeJsonValue;
  attachments?: NativeJsonValue;
  metadata?: NativeJsonValue;
}

export interface SequenceFactsOptions {
  namespacePrefix?: string;
  width?: number | bigint;
}

export interface WorkflowIdFactsOptions {
  namespacePrefix?: string;
  timestampMs?: number | bigint;
  randomnessHex?: string;
}

export interface EnsureSeedMirrorOptions {
  forceRefresh?: boolean;
  lineName?: string;
}

export interface LandSubmitOptions {
  targetLine: string;
  mode: string;
  patchsetId?: string;
  remoteName?: string;
}

export interface LandReceiptOptions {
  remoteName?: string;
  repoName?: string;
}

export interface LandRetryOptions extends LandReceiptOptions {
  reason?: string;
}

export interface ApplyPhaseOptions {
  detail?: string;
  resumedFromAuthoritativeState?: boolean;
}

export interface WorkflowReadyDirectOptions {
  snapshotMessage?: string;
  summary?: string;
  tests?: string;
  lint?: string;
  security?: string;
  license?: string;
  authorMode?: string;
  model?: string;
  remoteName?: string;
}

export interface WorkflowLandDirectOptions {
  reviewMessage?: string;
  remoteName?: string;
  reviewer?: string;
}

export interface WorkflowProjectionContext {
  root?: string;
  configPath?: string;
  worktreeConfigPath?: string;
  workspaceDir?: string;
  taskReview?: boolean;
}

export interface CreateWorkflowReleaseInput {
  releaseId: string;
  repoName: string;
  version: string;
  lineName: string;
  snapshotId: string;
  manifestHash: string;
  profile: string;
  packageName?: string;
  packageVersion?: string;
  packageRequiresPython?: string;
  status?: string;
  checksJson?: string;
  artifactsJson?: string;
  formulaJson?: string;
  metadataJson?: string;
}

export interface UpdateWorkflowReleaseOptions {
  status?: string;
  checksJson?: string;
  checksSet?: boolean;
  artifactsJson?: string;
  artifactsSet?: boolean;
  formulaJson?: string;
  formulaSet?: boolean;
  metadataJson?: string;
  metadataSet?: boolean;
}

export interface ReleaseCheckSdkOptions {
  testsCommand?: string;
  skipTestsReason?: string;
}

export interface ReleaseAdapterOptions {
  lineName?: string;
  target?: string;
}

export interface ReleaseCandidateOptions {
  lineName?: string;
  profile?: string;
  channel?: string;
  publicSourceRoot?: string;
}

export interface ReleaseCheckOptions {
  receipts?: string;
  publicSourceRoot?: string;
  testsCommand?: string;
  skipTestsReason?: string;
}

export interface ReleaseBuildOptions {
  receipts?: string;
  publicSourceRoot?: string;
  nativeMatrixDir?: string;
  nativeCommandDir?: string;
}

export interface ReleaseNativeSourceInput {
  releaseId: string;
  target: string;
  sourceDir: string;
  runner: string;
  runnerImage: string;
  rustToolchain: string;
  rustcPath: string;
}

export interface ReleaseShowOptions {
  remoteName?: string;
  publicSourceRoot?: string;
}

export interface ExternalUpdateOptions {
  name?: string;
  snapshot?: string;
  latest?: boolean;
  locked?: boolean;
  validate?: boolean;
  noRecursive?: boolean;
}

export interface RemoteHeadRecoveryOptions {
  remoteName?: string;
  jobs?: number | bigint;
  apply?: boolean;
}

export interface NativeAddon {
  readonly CODE_REVIEW_SUMMARY_TEMPLATE: string;
  readonly CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE: string;
  readonly CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND: string;
  readonly DEFAULT_ID_NAMESPACE_PREFIX: string;
  readonly LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX: string;
  readonly REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX: string;
  readonly RUNTIME_DATA_ENV: string;
  readonly LEGACY_SERVER_DATA_ENV: string;
  readonly CONTRACT_VERSION: string;
  readonly PLAN_CONTRACT_VERSION: string;
  readonly TARGET_TRIPLE: string;
  remoteRecoverHead(repoRoot: string, options?: RemoteHeadRecoveryOptions | null): NativeJsonValue;
  formatRemoteRecoverHead(payload: NativeJsonValue): string;
  parseSnapshotAuthoringReference(reference: string): string;
  workflowIdFamilies(): string[];
  workflowTaskChangeOriginNamespacePrefixes(): string[];
  policyRequirementFlags(): string[];

  externalStatus(repoRoot: string): NativeJsonValue;
  externalDoctor(repoRoot: string, failOnBlocking?: boolean | null): WorkflowPhaseResult;
  externalUpdate(repoRoot: string, options?: ExternalUpdateOptions | null): NativeJsonValue;
  externalLink(repoRoot: string, name: string, path: string): NativeJsonValue;
  externalUnlink(repoRoot: string, name: string): NativeJsonValue;
  teamContacts(repoRoot: string): WorkflowPhaseResult;
  teamSessionEnsure(repoRoot: string, agent: number | bigint, role?: number | bigint | null): WorkflowPhaseResult;
  teamSessionAcknowledge(repoRoot: string, activation: string): WorkflowPhaseResult;
  formatTeamContacts(payload: NativeJsonValue): string;
  formatTeamSessionEnsure(payload: NativeJsonValue, agent: number | bigint): string;
  formatTeamSessionAcknowledge(payload: NativeJsonValue): string;
  binaryDbUpgradeU64Seconds(repoRoot: string, generationRoot: string, sourceTimeWidth: string, activate?: boolean | null): NativeJsonValue;
  binaryDbAgentPool50(repoRoot: string, generationRoot: string, activate?: boolean | null): NativeJsonValue;
  formatBinaryDbUpgradeU64Seconds(payload: NativeJsonValue): string;
  formatBinaryDbAgentPool50(payload: NativeJsonValue): string;
  formatExternalStatus(payload: NativeJsonValue): string;
  formatExternalDoctor(payload: NativeJsonValue): string;
  formatExternalUpdate(payload: NativeJsonValue): string;
  formatExternalLink(payload: NativeJsonValue): string;
  formatExternalUnlink(payload: NativeJsonValue): string;

  releaseAdapterCheck(repoRoot: string, version: string, options?: ReleaseAdapterOptions | null): NativeJsonValue;
  releaseAdapterBuild(repoRoot: string, version: string, options?: ReleaseAdapterOptions | null): NativeJsonValue;
  releaseCandidateCreate(repoRoot: string, version: string, options?: ReleaseCandidateOptions | null): NativeJsonValue;
  releaseCheck(repoRoot: string, releaseId: string, options?: ReleaseCheckOptions | null): NativeJsonValue;
  releaseBuild(repoRoot: string, releaseId: string, options?: ReleaseBuildOptions | null): NativeJsonValue;
  releaseNativeSource(repoRoot: string, request: ReleaseNativeSourceInput): NativeJsonValue;
  releaseNativeBundle(repoRoot: string, releaseId: string, nativeMatrixDir: string): NativeJsonValue;
  releasePackage(repoRoot: string, releaseId: string, channel: string, publicSourceRoot?: string | null): NativeJsonValue;
  releaseFormula(repoRoot: string, releaseId: string, name: string, pythonFormula?: string | null): NativeJsonValue;
  releaseShow(repoRoot: string, releaseId: string, options?: ReleaseShowOptions | null): NativeJsonValue;
  releasePublish(repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;
  releasePromote(repoRoot: string, releaseId: string, channel: string, publicSourceRoot?: string | null): NativeJsonValue;
  formatRelease(payload: NativeJsonValue): string;

  createWorkflowRelease(repoRoot: string, input: CreateWorkflowReleaseInput): NativeJsonValue;
  listWorkflowReleases(repoRoot: string): NativeJsonValue;
  getWorkflowRelease(repoRoot: string, releaseId: string): NativeJsonValue;
  updateWorkflowRelease(repoRoot: string, releaseId: string, options?: UpdateWorkflowReleaseOptions | null): NativeJsonValue;
  taskWorkflowReleaseCandidateCreate(repoRoot: string, version: string, line: string, profile: string): NativeJsonValue;
  taskWorkflowReleaseCheck(repoRoot: string, releaseId: string, options?: ReleaseCheckSdkOptions | null): NativeJsonValue;
  taskWorkflowReleaseBuild(repoRoot: string, releaseId: string): NativeJsonValue;
  taskWorkflowReleaseFormula(repoRoot: string, releaseId: string, name: string): NativeJsonValue;
  taskWorkflowReleaseShow(repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowReleasePublish(repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;

  projectWorkflowLandedReadModel(context: WorkflowProjectionContext, state: NativeJsonValue): NativeJsonValue;
  projectWorkflowLandFullReadModel(context: WorkflowProjectionContext, state: NativeJsonValue, applyOwnedContinuation: boolean): NativeJsonValue;
  projectWorkflowReadyReadModel(context: WorkflowProjectionContext, state: NativeJsonValue, ignoreWorkspaceAuthoring: boolean, patchsetIsAuthoritative: boolean, applyOwnedContinuation: boolean): NativeJsonValue;

  buildLinkedTaskLookupPayload(taskLinksByItemRows?: NativeJsonValue | null, tasksByPlanRows?: NativeJsonValue | null): NativeJsonValue;
  buildLinkedChangeLookupPayload(changeLinksByTaskRows?: NativeJsonValue | null): NativeJsonValue;
  buildTaskTrackingTitlePayload(task: NativeJsonValue): NativeJsonValue;
  buildTaskTrackingMetadataPayload(task: NativeJsonValue, authorMode: string, trackingPolicy: string): NativeJsonValue;
  buildTaskAuditVerdictPayload(task: NativeJsonValue, changeRows: NativeJsonValue, targetLine: string): NativeJsonValue;
  workflowLandedFacts(state: NativeJsonValue): NativeJsonValue;
  workflowLandFullFacts(state: NativeJsonValue): NativeJsonValue;
  workflowReadyFacts(state: NativeJsonValue): NativeJsonValue;
  workflowLandPhaseFacts(state: NativeJsonValue, readyState: NativeJsonValue): NativeJsonValue;
  projectWorkflowLandPhaseReadModel(state: NativeJsonValue, readyState: NativeJsonValue): NativeJsonValue;
  workflowRemoteMutationReceipt(input: RemoteMutationReceiptInput): NativeJsonValue;
  workflowRemoteActionMutationReceipts(code: string, result: NativeJsonValue): NativeJsonValue[];
  workflowApplyPhasePayload(phase: string, code: string, options?: ApplyPhaseOptions | null): NativeJsonValue;
  workflowApplyPhaseSummary(applyPhase: NativeJsonValue): string | null;
  workflowMutationReceiptSummary(receipt: NativeJsonValue): string;
  workflowAppliedActionSummary(action: NativeJsonValue): string;
  workflowReadyPayloadDirect(repoRoot: string, changeId: string, remoteName?: string | null): NativeJsonValue;
  workflowLandPayloadDirect(repoRoot: string, changeId: string, remoteName?: string | null): NativeJsonValue;
  workflowReadyApplyDirect(repoRoot: string, changeId: string, options?: WorkflowReadyDirectOptions | null): NativeJsonValue;
  workflowLandApplyDirect(repoRoot: string, changeId: string, options?: WorkflowLandDirectOptions | null): NativeJsonValue;

  taskWorkflowTimestampFacts(now?: string | null): NativeJsonValue;
  taskWorkflowSequenceIdentityFacts(family: string, number: number | bigint, options?: SequenceFactsOptions | null): NativeJsonValue;
  taskWorkflowWorkflowIdFacts(family: string, options?: WorkflowIdFactsOptions | null): NativeJsonValue;
  taskWorkflowRuntimeSelectionFacts(overrides?: NativeJsonValue | null): NativeJsonValue;
  taskWorkflowReadUtf8TextFile(pathValue: string): string;
  taskWorkflowReadJsonFile(pathValue: string): NativeJsonValue;
  taskWorkflowReadBinaryFile(pathValue: string): Buffer;
  taskWorkflowResolveRepoArtifactPath(repoRoot: string, pathValue: string, allowMissing?: boolean | null): ResolvedRepoArtifactPath;
  taskRemoteChangeLineagePayload(baseLine: string, lineRow?: NativeJsonValue | null): NativeJsonValue;
  taskWorkflowResolveTaskWorktreeLocation(repoRoot: string, worktreeName: string, debugProbeOverride?: NativeJsonValue | null): NativeJsonValue;
  taskWorkflowResolveMainSeedMirrorLocation(repoRoot: string, seedName: string): NativeJsonValue;
  taskWorkflowEnsureMainSeedMirror(repoRoot: string, options?: EnsureSeedMirrorOptions | null): NativeJsonValue;
  taskWorkflowEnsureStatusManifest(repoRoot: string, snapshotId: string): NativeJsonValue;
  taskWorkflowQueueSummary(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowTaskAudit(repoRoot: string, taskId: string, scope?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowTaskAbandon(repoRoot: string, taskId: string, scope?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowTaskLand(repoRoot: string, taskOrChangeId: string, scope?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowLandSubmit(repoRoot: string, changeId: string, options: LandSubmitOptions): NativeJsonValue;
  taskWorkflowLandShow(repoRoot: string, submissionId: string, options?: LandReceiptOptions | null): NativeJsonValue;
  taskWorkflowLandRetry(repoRoot: string, submissionId: string, options?: LandRetryOptions | null): NativeJsonValue;

  extractCodexUsageJsonl(usageJsonlPath: string, usageScope: string): NativeJsonValue;
  extractCodexUsageBundleJsonl(usageJsonlPaths: string[], options?: UsageBundleOptions | null): NativeJsonValue;
  projectRepoRetireRuntimeBlockers(payload: NativeJsonValue): NativeJsonValue;
  languageBindingInfo(): LanguageBindingInfo;
  transportEnvelopeIrVersion(): string;
  transportEnvelopeSchema(): NativeJsonValue;
  runtimeBindingStateIrVersion(): string;
  runtimeBindingStateSchema(): NativeJsonValue;
  runtimeBindingStateDefaultPayload(): NativeJsonValue;
  runtimeBindingStateNormalizeDocument(payload: NativeJsonValue): NativeJsonValue;
  workerManifestIrVersion(): string;
  workerManifestSchema(): NativeJsonValue;
  workerManifestDefaultConfig(): NativeJsonValue;
  workerManifestNormalizeDocument(payload: NativeJsonValue, path?: string | null): NativeJsonValue;
  workerManifestSelectTelegramWorker(config: NativeJsonValue, requestedName?: string | null): NativeJsonValue;
  transportEnvelopeBuildBindingMetadata(input: TransportBindingMetadataInput): NativeJsonValue;
  transportEnvelopeBuildEventEnvelope(input: TransportEventInput): NativeJsonValue;
  transportEnvelopeBuildReplyEnvelope(input: TransportReplyInput): NativeJsonValue;
  transportEnvelopeCompactEventEnvelope(envelope: NativeJsonValue): NativeJsonValue;
  transportEnvelopeCompactReplyEnvelope(envelope: NativeJsonValue): NativeJsonValue;

  HttpTaskRemote: NativeHttpTaskRemoteConstructor;
  HttpWorkflowCloseoutRemote: NativeHttpWorkflowCloseoutRemoteConstructor;

  readonly DEFAULT_REQUEST_TIMEOUT_SECONDS: 30;
  readonly LONG_RUNNING_REQUEST_TIMEOUT_SECONDS: 1800;
  PlanRemoteTransportConfig: NativePlanRemoteTransportConfigConstructor;
  PlanHttpClientManager: NativePlanHttpClientManagerConstructor;
  TaskWorkflowHttpClientManager: NativeTaskWorkflowHttpClientManagerConstructor;

  joinPath(baseUrl: string, path: string): string;
  buildQueryString(params: Record<string, string>): string | null;
  normalizeRemoteText(value?: string | null): string | null;
  buildListPlansRequest(baseUrl: string, repoName: string, artifactPath?: string | null): PlanHttpRequest;
  buildGetPlanRequest(baseUrl: string, planId: string): PlanHttpRequest;
  buildListPlanRevisionsRequest(baseUrl: string, planId: string): PlanHttpRequest;
  buildGetPlanRevisionRequest(baseUrl: string, planId: string, planRevisionId: string): PlanHttpRequest;
  buildCreatePlanRequest(baseUrl: string, input: CreatePlanInput): PlanHttpRequest;
  buildRevisePlanRequest(baseUrl: string, input: RevisePlanInput): PlanHttpRequest;
  buildUpdatePlanStatusRequest(baseUrl: string, planId: string, status: string): PlanHttpRequest;
  buildPutPlanRevisionArtifactsRequest(baseUrl: string, planId: string, planRevisionId: string, artifacts: NativeJsonValue[]): PlanHttpRequest;
  listPlans(baseUrl: string, repoName: string, artifactPath?: string | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  getPlan(baseUrl: string, planId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanRevisions(baseUrl: string, planId: string, options?: PlanHttpOptions | null): NativeJsonValue[];
  getPlanRevision(baseUrl: string, planId: string, planRevisionId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  resolveTaskPlanLinkage(baseUrl: string, repoName: string, linkage?: TaskPlanLinkageOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanIdsMatchingContains(baseUrl: string, repoName: string, containsTerms: string[], options?: PlanHttpOptions | null): NativeJsonValue[];
  createPlan(baseUrl: string, input: CreatePlanInput, options?: PlanHttpOptions | null): NativeJsonValue;
  revisePlan(baseUrl: string, input: RevisePlanInput, options?: PlanHttpOptions | null): NativeJsonValue;
  updatePlanStatus(baseUrl: string, planId: string, status: string, options?: PlanHttpOptions | null): NativeJsonValue;
  putPlanRevisionArtifacts(baseUrl: string, planId: string, planRevisionId: string, artifacts: NativeJsonValue[], options?: PlanHttpOptions | null): NativeJsonValue;
  createPlanningSession(baseUrl: string, planId: string, input?: CreatePlanningSessionOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanningSessions(baseUrl: string, planId: string, status?: string | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  getPlanningSession(baseUrl: string, planningSessionId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  appendPlanningSessionEvent(baseUrl: string, planningSessionId: string, eventType: string, payload?: NativeJsonValue | null, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanningSessionEvents(baseUrl: string, planningSessionId: string, input?: PlanningSessionEventsOptions | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  joinPlanningSession(baseUrl: string, planningSessionId: string, input?: JoinPlanningSessionOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  promotePlanningSession(baseUrl: string, planningSessionId: string, input: PromotePlanningSessionInput, options?: PlanHttpOptions | null): NativeJsonValue;
  closePlanningSession(baseUrl: string, planningSessionId: string, status?: string | null, options?: PlanHttpOptions | null): NativeJsonValue;

  planPackSubstrateBuildPackMembers(blobItems: NativeJsonValue, options?: BuildPackMembersOptions | null): NativeJsonValue;
  planPackSubstrateBuildTreePackMembers(treeRows: NativeJsonValue, treeEntryRows: NativeJsonValue): NativeJsonValue;
  planPackSubstrateBuildGitBinaryDelta(baseData: Buffer, targetData: Buffer): Buffer;
  planPackSubstrateBuildGitBinaryDeltaMember(entryName: string, blobId: string, baseBlobId: string, baseData: Buffer, targetData: Buffer, chainDepth: number | bigint): NativeJsonValue;
  planPackSubstrateApplyGitBinaryDelta(baseData: Buffer, deltaData: Buffer): Buffer;
  planPackSubstrateApplyPackDelta(baseData: Buffer, deltaData: Buffer, algorithm: string): Buffer;
  planPackSubstrateWritePackArchive(packPath: string, packId: string, createdAt: string, members: NativeJsonValue): NativeJsonValue;
  planPackSubstrateWriteTreePackArchive(packPath: string, packId: string, createdAt: string, members: NativeJsonValue): NativeJsonValue;
  planPackSubstrateReadPackIndex(packPath: string): NativeJsonValue;
  planPackSubstrateReadTreePackIndex(packPath: string): NativeJsonValue;
  planPackSubstratePackHasEntry(packPath: string, entryName: string): boolean;
  planPackSubstrateReadTreePackTree(packPath: string, treeId: string): NativeJsonValue;
  planPackSubstrateSummarizePackArchives(root: string, packRows: NativeJsonValue): NativeJsonValue;
  planPackSubstrateSummarizeTreePackArchives(root: string, packRows: NativeJsonValue): NativeJsonValue;
  planPackSubstrateTreePackManifestPath(packPath: string, entryName: string): string;
  planPackSubstrateReadTreePackTreeByOrdinal(packPath: string, entryOrdinal: number | bigint): NativeJsonValue;
  planPackSubstrateBuildStorageValidationSummary(options: StorageValidationOptions): NativeJsonValue;
  planPackSubstrateBuildTreeRecords(fileEntries: NativeJsonValue): NativeJsonValue;
  planPackSubstrateBuildSnapshotId(options: BuildSnapshotIdOptions): NativeJsonValue;

  artifactBlobId(markdown: string): string;
  artifactCandidatesOpen(candidates: NativeJsonValue): NativeJsonValue;
  indexPlansByArtifactPath(plans: NativeJsonValue): NativeJsonValue;
  indexPlansByArtifactIdentity(plans: NativeJsonValue): NativeJsonValue;
  planArtifactIdentity(artifactPath: string, artifactSelector?: string | null): NativeJsonValue;
  planArtifactIdentityLabel(artifactPath: string, artifactSelector?: string | null): string;
  openGenericPlansMatchingBlobId(plans: NativeJsonValue, blobId: string): NativeJsonValue;
  openPlansMatchingSelector(plans: NativeJsonValue, selector: string): NativeJsonValue;
  localPlanFullyPublished(plan: NativeJsonValue): boolean;
  planHeadsEquivalent(left: NativeJsonValue, right: NativeJsonValue): boolean;
  planMatchesSyncArtifact(plan: NativeJsonValue, artifact: NativeJsonValue, requireTitleMatch?: boolean | null): boolean;
  planFilesystemNormalizeMarkdownArtifactPath(pathValue: string): string;
  planFilesystemIsMarkdownArtifactPath(pathValue: string): boolean;
  planFilesystemIsLineageOnlyMarkdownArtifactPath(pathValue: string): boolean;
  planFilesystemReadUtf8TextFile(pathValue: string): string;
  planFilesystemReadJsonFile(pathValue: string): NativeJsonValue;
  planFilesystemReadBinaryFile(pathValue: string): Buffer;
  planFilesystemPathIsProjectedOutForWorkspace(repoRoot: string, relPath: string, isWorktree?: boolean | null): boolean;
  planFilesystemWorkspacePathIsIgnored(repoRoot: string, pathValue: string, ignoreRulesText?: string | null): boolean;
  planFilesystemListVisibleWorkspacePaths(repoRoot: string, options?: VisibleWorkspaceOptions | null): string[];
  planFilesystemListVisibleMarkdownArtifactPaths(repoRoot: string, options?: VisibleWorkspaceOptions | null): string[];
  planFilesystemZipArchiveHasMember(pathValue: string, entryName: string): boolean;
  planFilesystemReadZipArchiveMember(pathValue: string, entryName: string): Buffer;
  planFilesystemResolveRepoArtifactPath(repoRoot: string, pathValue: string, allowMissing?: boolean | null): ResolvedRepoArtifactPath;
  repoStateReadRepoConfigJson(pathValue: string): NativeJsonValue;
  repoStateWriteRepoConfigJson(pathValue: string, payload: NativeJsonValue): void;
  repoStateReadWorktreeConfigJson(pathValue: string): NativeJsonValue;
  repoStateWriteWorktreeConfigJson(pathValue: string, payload: NativeJsonValue): void;
  repoStateReadWorktreeMetadataJson(pathValue: string): NativeJsonValue;
  repoStateWriteWorktreeMetadataJson(pathValue: string, payload: NativeJsonValue): void;

  currentCoreSourceFingerprint(coreRepoRoot: string): string;
  currentServerSourceFingerprint(serverCoreRepoRoot: string): string;
  currentSourceNativeCacheContract(request: CurrentSourceCacheRequest): NativeJsonValue;
  currentSourceCacheMarkBuilding(request: CurrentSourceCacheRequest, sourceMtimeNs: number | bigint, extraJson?: string | null): NativeJsonValue;
  currentSourceCacheActivate(request: CurrentSourceCacheRequest, sourceMtimeNs: number | bigint, options?: CurrentSourceCacheActivateOptions | null): NativeJsonValue;
  currentSourceCacheReleaseLease(leasePath: string, namespaceRoot: string, removeUnleasedReady?: boolean | null): NativeJsonValue;
  currentSourceCachePrune(namespaceRoot: string, options?: CurrentSourceCachePruneOptions | null): NativeJsonValue;
  currentSourceCacheSeedCanonical(request: CurrentSourceCacheRequest, repoRoot: string, canonicalRepoRoot: string, coreSourceMtimeNs: number | bigint): NativeJsonValue;
  currentSourceCacheExtensionFresh(request: CurrentSourceExtensionFreshnessRequest): NativeJsonValue;
  currentSourceCacheBinaryFresh(request: CurrentSourceBinaryFreshnessRequest): NativeJsonValue;
  currentSourceCacheCoreFingerprint(repoRoot: string): NativeJsonValue;
  currentSourceCacheServerFingerprint(repoRoot: string): NativeJsonValue;
  currentSourceCacheCoreMtime(repoRoot: string): NativeJsonValue;
  currentSourceCacheServerMtime(repoRoot: string): NativeJsonValue;
  formatCurrentSourceCacheContract(payload: NativeJsonValue): string;
  formatCurrentSourceCacheMarkBuilding(payload: NativeJsonValue): string;
  formatCurrentSourceCacheActivate(payload: NativeJsonValue): string;
  formatCurrentSourceCacheReleaseLease(payload: NativeJsonValue): string;
  formatCurrentSourceCachePrune(payload: NativeJsonValue): string;
  formatCurrentSourceCacheSeedCanonical(payload: NativeJsonValue): string;
  formatCurrentSourceCacheExtensionFresh(payload: NativeJsonValue): string;
  formatCurrentSourceCacheBinaryFresh(payload: NativeJsonValue): string;
  formatCurrentSourceCacheCoreFingerprint(payload: NativeJsonValue): string;
  formatCurrentSourceCacheServerFingerprint(payload: NativeJsonValue): string;
  formatCurrentSourceCacheCoreMtime(payload: NativeJsonValue): string;
  formatCurrentSourceCacheServerMtime(payload: NativeJsonValue): string;
  extractPlanItems(markdown?: string | null): ParsedPlanItem[];
  listPlanSectionRefs(markdown?: string | null): ParsedPlanReference[];
  extractPlanSection(markdown?: string | null, planRef?: string | null): ExtractedPlanSection | null;
  findPlanItem(markdown?: string | null, planItemRef?: string | null): ParsedPlanItem | null;
  normalizePlanItems(items?: PlanItemInput[] | null): ParsedPlanItem[];
  findPlanItemInItems(items?: PlanItemInput[] | null, planItemRef?: string | null): ParsedPlanItem | null;
  normalizeTaskWorkflowHttpCompatibility(payloadJson: string): NativeJsonValue;
  normalizeTaskWorkflowHttpReadiness(payloadJson: string): NativeJsonValue;
  planItemsPayload(plan: NativeJsonValue, revision?: NativeJsonValue | null): NativeJsonValue;
  localPlanPublishShadow(plan?: NativeJsonValue | null): NativeJsonValue;
  planDispatchSummary(plan: NativeJsonValue, options?: PlanDispatchOptions | null): NativeJsonValue;
  computeTaskableItems(plan: NativeJsonValue, options?: PlanDispatchOptions | null): NativeJsonValue;
  validateDispatchLegality(plan: NativeJsonValue, planItemRef?: string | null, options?: PlanDispatchOptions | null): NativeJsonValue;
  planTaskLinkIndexes(tasks?: NativeJsonValue | null): PlanTaskLinkIndexes;
  planCandidatesPayload(summaries?: NativeJsonValue | null, options?: PlanCandidatesPayloadOptions | null): NativeJsonValue;

  policyProfileNames(): string[];
  authorModeValues(): string[];
  policyContentClassValues(): string[];
  policyAuthorClassValues(): string[];
  normalizeAuthorMode(value: string): string;
  missingCodeReviewSummarySections(value?: string | null): string[];
  isStructuredCodeReviewSummary(value?: string | null): boolean;
  renderCodeReviewSummaryTemplate(style?: string | null): string;
  codeReviewSummaryRequirementText(value?: string | null): string;
  derivePolicyContentClass(changedPaths?: NativeJsonValue | null): string;
  derivePolicyAuthorClass(authorMode?: string | null): string | null;
  buildMinimumProvenance(authorMode: string, modelName?: string | null): [NativeJsonValue, NativeJsonValue];
  policyProfile(name: string): NativeJsonValue;
  normalizePolicy(policy?: NativeJsonValue | null, fallbackProfile?: string | null): NativeJsonValue;
  resolveEffectivePolicy(policy?: NativeJsonValue | null, options?: EffectivePolicyOptions | null): NativeJsonValue;
  policyToYaml(policy?: NativeJsonValue | null, fallbackProfile?: string | null): string;
  parsePolicyYaml(text: string, fallbackProfile?: string | null): NativeJsonValue;
  runtimeDataEnvValue(): string | null;
  runtimeDataEnvName(): string | null;
  resolveRuntimeDataRootWithSource(path?: string | null): [string, string];
  resolveRuntimeDataRoot(path?: string | null): string;
  resolveServerRuntimeRootWithSource(path?: string | null): [string, string];
  resolveServerRuntimeRoot(path?: string | null): string;
  encodeRefName(name: string): string;
  decodeRefName(name: string): string;
  normalizeTaskStatus(value?: string | null): string | null;
  normalizeOptionalText(value?: string | null): string | null;
  utcNow(): string;
  generateNamespacedWorkflowId(family: string, namespacePrefix?: string | null): string;
  normalizeIdNamespacePrefix(value?: string | null, defaultPrefix?: string | null): string;
  workflowIdTokens(family: string, options?: WorkflowIdOptions | null): string[];
  workflowOriginNamespacePrefix(originPrefix: string, namespacePrefix?: string | null): string;
  workflowIdNamespacePrefixCandidates(options?: WorkflowNamespaceOptions | null): string[];
  workflowIdNamespacePrefixForValue(family: string, value?: string | null, options?: WorkflowNamespaceOptions | null): string | null;
  workflowIdMatchesAnyNamespacePrefix(family: string, value?: string | null, options?: WorkflowNamespaceOptions | null): boolean;
  derivePatchsetId(changeId: string, patchsetNumber: number | bigint, namespacePrefix?: string | null): string;
  generateWorkflowId(prefix: string): string;
  taskStatusDetails(value?: string | null): TaskStatusDetails;
  normalizePublicationState(value?: string | null): string | null;
  publicationStateHasUnpublishedHead(value?: string | null): boolean;
  workflowIdToken(family: string, namespacePrefix?: string | null): string;
  workflowIdMatches(family: string, value?: string | null, options?: WorkflowIdOptions | null): boolean;
  generateNamespacedSequenceId(family: string, number: number | bigint, options?: WorkflowSequenceOptions | null): string;
  workflowSuccessEnvelope(kind: string, value?: string | null): WorkflowResultEnvelope;
  workflowErrorEnvelope(kind: string, code: string, message: string, detail?: string | null): WorkflowResultEnvelope;
  parsePlanMarkdown(markdown?: string | null): ParsedPlan;
  extractPlanRefs(parsedPlan: ParsedPlanInput): PlanRefsResult;
  computeSyncPruneDecisions(scope?: string | null, trackedArtifacts?: string[] | null, syncedArtifacts?: string[] | null): NativeJsonValue;
  diffSnapshotManifests(oldFiles: NativeJsonValue, newFiles: NativeJsonValue, options?: ManifestIdentityOptions | null): NativeJsonValue;
  snapshotDiffFromManifests(oldFiles: NativeJsonValue, newFiles: NativeJsonValue, options?: ManifestTextDiffOptions | null): NativeJsonValue;
  gitMigrate(root: string, stagingRoot: string, apply?: boolean | null): NativeJsonValue;
  gitRecover(root: string, apply?: boolean | null): NativeJsonValue;
  gitImport(root: string, source: string, options?: GitImportOptions | null): NativeJsonValue;
  gitExport(root: string, target: string, options?: GitExportOptions | null): NativeJsonValue;
  gitRestore(root: string, generation: string, gitObject: string, target: string): NativeJsonValue;
  gitMirror(root: string, endpoint: string, direction: string, dryRun?: boolean | null): NativeJsonValue;
  formatGitMigrate(payload: NativeJsonValue): string;
  formatGitRecover(payload: NativeJsonValue): string;
  formatGitImport(payload: NativeJsonValue): string;
  formatGitExport(payload: NativeJsonValue): string;
  formatGitRestore(payload: NativeJsonValue): string;
  formatGitMirror(payload: NativeJsonValue): string;
  gcStats(root: string): NativeJsonValue;
  gcValidate(root: string): NativeJsonValue;
  gcPrune(root: string, apply?: boolean | null): NativeJsonValue;
  gcValidateExitCode(payload: NativeJsonValue): number;
  formatGcStats(payload: NativeJsonValue): string;
  formatGcValidate(payload: NativeJsonValue): string;
  formatGcPrune(payload: NativeJsonValue): string;
  worktreeShow(root: string, name?: string | null): NativeJsonValue;
  worktreePath(root: string, name?: string | null): NativeJsonValue;
  worktreeRestore(root: string, options?: WorktreeRestoreOptions | null): NativeJsonValue;
  worktreeCleanup(root: string, options?: WorktreeCleanupCommandOptions | null): NativeJsonValue;
  worktreePruneStale(root: string, options?: WorktreePruneCommandOptions | null): NativeJsonValue;
  worktreeSync(root: string, options?: WorktreeSyncCommandOptions | null): NativeJsonValue;
  worktreeRecreate(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  worktreeRecoverTask(root: string, taskId: string, options?: WorktreeRecoverTaskOptions | null): NativeJsonValue;
  worktreeRestoreOwnedHead(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  worktreeRebase(root: string, options?: WorktreeRebaseCommandOptions | null): NativeJsonValue;
  worktreeRemove(root: string, names: string[], options?: WorktreeRemoveCommandOptions | null): NativeJsonValue;
  formatWorktreeStatus(payload: NativeJsonValue, verbose?: boolean | null): string;
  formatWorktreePath(payload: NativeJsonValue, shellOutput?: boolean | null): string;
  formatWorktreeShow(payload: NativeJsonValue): string;
  formatWorktreeList(payload: NativeJsonValue): string;
  formatWorktreeDoctor(payload: NativeJsonValue): string;
  formatWorktreeCleanupCandidates(payload: NativeJsonValue): string;
  formatWorktreeCleanup(payload: NativeJsonValue): string;
  formatWorktreePruneStale(payload: NativeJsonValue): string;
  formatWorktreeSync(payload: NativeJsonValue): string;
  formatWorktreeRestore(payload: NativeJsonValue): string;
  formatWorktreeRecreate(payload: NativeJsonValue): string;
  formatWorktreeRecoverTask(payload: NativeJsonValue): string;
  formatWorktreeRestoreOwnedHead(payload: NativeJsonValue): string;
  formatWorktreeRebase(payload: NativeJsonValue): string;
  formatWorktreeRemove(payload: NativeJsonValue, allStale: boolean, dryRun: boolean, multiple: boolean): string;
  projectWorktreePath(payload: NativeJsonValue): NativeJsonValue;
  worktreeSyncExitCode(payload: NativeJsonValue, allWorktrees: boolean): number;
  pullRepository(root: string, options?: PullOptions | null): NativeJsonValue;
  pushRepository(root: string, options?: RemoteLineOptions | null): NativeJsonValue;
  taskWorkflowPull(root: string, options?: PullOptions | null): NativeJsonValue;
  taskWorkflowPush(root: string, options?: RemoteLineOptions | null): NativeJsonValue;
  taskWorkflowUploadSnapshotChain(root: string, snapshotId: string, options?: SnapshotUploadOptions | null): NativeJsonValue;
  formatPull(payload: NativeJsonValue): string;
  formatPush(payload: NativeJsonValue): string;
  projectRemoteSync(payload: NativeJsonValue): NativeJsonValue;
  workspaceDiff(repoRoot: string, paths?: string[] | null): NativeJsonValue;
  renderWorkspaceDiffNames(payload: NativeJsonValue): string;
  renderWorkspaceDiffStat(payload: NativeJsonValue): string;
  renderWorkspaceDiffText(payload: NativeJsonValue): string;
  taskWorkflowBlame(repoRoot: string, path: string, options?: BlameOptions | null): NativeJsonValue;
  formatBlame(payload: NativeJsonValue): string;
  formatAttestPut(payload: NativeJsonValue): string;
  formatAttestShow(payload: NativeJsonValue): string;
  formatPolicyEval(payload: NativeJsonValue): string;
  formatPolicyShow(payload: NativeJsonValue): string;
  formatPolicyWaive(payload: NativeJsonValue): string;
  projectPolicyShow(payload: NativeJsonValue): NativeJsonValue;
  validateReviewPatchsetSelector(value: string): string;
  validatePublicPatchsetArgument(value: string): string;
  reviewShow(repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  reviewTeamRequest(repoRoot: string, reference: string, reviewerGroups: string[], options?: ReviewRequestOptions | null): NativeJsonValue;
  reviewTeamApprove(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTeamRequestChanges(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTeamComment(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTeamDefer(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTaskApprove(repoRoot: string, reference: string, patchsetSelector: string, message: string, remoteName?: string | null): ReviewTaskOperationResult;
  reviewTaskRequestChanges(repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  reviewTaskComment(repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  reviewTaskDefer(repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  reviewCodeSubmit(repoRoot: string, reference: string, patchsetId: string, message: string, options?: ReviewCodeSubmitOptions | null): NativeJsonValue;
  formatReviewTaskApprove(operation: ReviewTaskOperationResult): string;
  formatReviewTaskRequestChanges(operation: ReviewTaskOperationResult): string;
  formatReviewTaskComment(operation: ReviewTaskOperationResult): string;
  formatReviewTaskDefer(operation: ReviewTaskOperationResult): string;
  formatReviewTeamRequest(payload: NativeJsonValue): string;
  formatReviewTeamApprove(payload: NativeJsonValue): string;
  formatReviewTeamRequestChanges(payload: NativeJsonValue): string;
  formatReviewTeamComment(payload: NativeJsonValue): string;
  formatReviewTeamDefer(payload: NativeJsonValue): string;
  formatReviewShow(payload: NativeJsonValue): string;
  formatReviewCodeSubmit(payload: NativeJsonValue): string;
  formatReviewCodeTemplate(payload: NativeJsonValue): string;
  projectReviewTaskOperation(operation: ReviewTaskOperationResult): NativeJsonValue;
  projectReviewShow(payload: NativeJsonValue): NativeJsonValue;
  tagCreate(repoRoot: string, name: string, message: string, snapshotId?: string | null): NativeJsonValue;
  tagList(repoRoot: string): NativeJsonValue[];
  tagShow(repoRoot: string, name: string): NativeJsonValue;
  tagDelete(repoRoot: string, name: string): NativeJsonValue;
  resolveSnapshotRef(repoRoot: string, value: string): string;
  formatTagCreate(payload: NativeJsonValue): string;
  formatTagList(payload: NativeJsonValue): string;
  formatTagShow(payload: NativeJsonValue): string;
  formatTagDelete(payload: NativeJsonValue): string;
  stashSave(repoRoot: string, options?: StashSaveOptions | null): NativeJsonValue;
  taskWorkflowStashList(repoRoot: string): NativeJsonValue;
  taskWorkflowStashShow(repoRoot: string, stashId: string): NativeJsonValue;
  stashApply(repoRoot: string, stashId: string, force?: boolean | null): NativeJsonValue;
  stashPop(repoRoot: string, stashId: string, force?: boolean | null): NativeJsonValue;
  stashDrop(repoRoot: string, stashId: string): NativeJsonValue;
  taskWorkflowStashDrop(repoRoot: string, stashId: string): NativeJsonValue;
  formatStashSave(payload: NativeJsonValue): string;
  formatStashList(payload: NativeJsonValue): string;
  formatStashShow(payload: NativeJsonValue): string;
  formatStashApply(payload: NativeJsonValue): string;
  formatStashPop(payload: NativeJsonValue): string;
  formatStashDrop(payload: NativeJsonValue): string;
  taskWorkflowAuthWhoami(repoRoot: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowAuthBindings(repoRoot: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowAuthGrant(repoRoot: string, actorIdentity: string, roles: string[], options?: RemoteRepositoryOptions | null): NativeJsonValue;
  formatAuthWhoami(payload: NativeJsonValue): string;
  formatAuthBindings(payload: NativeJsonValue): string;
  formatAuthGrant(payload: NativeJsonValue): string;
  taskWorkflowWorktreeGet(root: string, options?: WorktreeGetOptions | null): NativeJsonValue;
  taskWorkflowWorktreeStatus(root: string, options?: WorktreeStatusOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRestore(root: string, options?: WorktreeRestoreOptions | null): NativeJsonValue;
  taskWorkflowWorktreeList(root: string, options?: WorktreeRefreshOptions | null): NativeJsonValue;
  taskWorkflowWorktreeTouchUsage(root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  taskWorkflowWorktreeDoctor(root: string, options?: WorktreeRefreshOptions | null): NativeJsonValue;
  taskWorkflowWorktreeCleanupCandidates(root: string, options?: WorktreeCleanupCandidatesOptions | null): NativeJsonValue;
  taskWorkflowWorktreeCleanup(root: string, options?: WorktreeCleanupOptions | null): NativeJsonValue;
  taskWorkflowWorktreePruneStale(root: string, options?: WorktreeDryRunOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRemove(root: string, names: string[], options?: WorktreeRemoveOptions | null): NativeJsonValue;
  taskWorkflowWorktreeBindExisting(root: string, worktreeName: string, options?: WorktreeBindExistingOptions | null): NativeJsonValue;
  taskWorkflowWorktreeSync(root: string, options?: WorktreeSyncOptions | null): NativeJsonValue;
  taskWorkflowWorktreeSyncAll(root: string, options?: WorktreeMutationOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRecreate(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRestoreOwnedHead(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  taskWorkflowWorktreePreviewRebase(root: string, options?: WorktreeRebaseOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRebase(root: string, options?: WorktreeRebaseOptions | null): NativeJsonValue;
  taskWorkflowWorktreeContinueRebase(root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  taskWorkflowWorktreeAbortRebase(root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  WorkspaceCommandLock: NativeWorkspaceCommandLockConstructor;
  workspaceCommandLockPath(repoRoot: string): string;
  taskWorkflowSnapshotChain(repoRoot: string, snapshotId: string): NativeJsonValue;
  taskWorkflowSnapshotExists(repoRoot: string, snapshotId: string): boolean;
  taskWorkflowBlobReadBytes(repoRoot: string, blobId: string): Buffer;
  taskWorkflowBlobEnsureBytes(repoRoot: string, data: Buffer, pathHint?: string | null): string;
  taskWorkflowWorkspaceDelta(repoRoot: string, snapshotId?: string | null): NativeJsonValue;
  taskWorkflowWorkspaceRestore(repoRoot: string, options?: WorkspaceRestoreOptions | null): NativeJsonValue;
  taskWorkflowWorkspaceRestorePaths(repoRoot: string, paths: string[], options?: WorkspaceRestoreOptions | null): NativeJsonValue;
  taskWorkflowSnapshotDiff(repoRoot: string, older: string, newer: string, options?: SnapshotDiffOptions | null): NativeJsonValue;
  taskWorkflowSnapshotRevert(repoRoot: string, snapshotId: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  taskWorkflowSnapshotReplay(repoRoot: string, snapshotId: string, onto: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  taskWorkflowSnapshotCreateExplicit(repoRoot: string, repoName: string, lineName: string, options?: SnapshotExplicitCaptureOptions | null): NativeJsonValue;
  taskWorkflowSnapshotRestoreLines(repoRoot: string, snapshotId: string, path: string, options?: SnapshotRestoreLinesOptions | null): NativeJsonValue;
  snapshotDiff(repoRoot: string, olderSelector: string, newerSelector: string, options?: SnapshotDiffOptions | null): NativeJsonValue;
  snapshotRevert(repoRoot: string, selector: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  snapshotReplay(repoRoot: string, selector: string, onto?: string | null, options?: SnapshotMutationOptions | null): NativeJsonValue;
  formatSnapshotDiff(payload: NativeJsonValue): string;
  formatSnapshotRevert(payload: NativeJsonValue): string;
  formatSnapshotReplay(payload: NativeJsonValue): string;
  formatSnapshotRestoreLines(payload: NativeJsonValue): string;
  changeCreate(repoRoot: string, taskId: string, title: string, options?: ChangeCreateOptions | null): NativeJsonValue;
  changeShow(repoRoot: string, reference: string, options?: ScopeOptions | null): NativeJsonValue;
  changeRevert(repoRoot: string, reference: string, options?: ChangeOperationOptions | null): NativeJsonValue;
  changeReplay(repoRoot: string, reference: string, onto?: string | null, options?: ChangeOperationOptions | null): NativeJsonValue;
  changeClose(repoRoot: string, reference: string, options?: ScopeOptions | null): NativeJsonValue;
  changePublish(repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  patchsetPublish(repoRoot: string, reference: string, summary: string, options?: PatchsetPublishOptions | null): NativeJsonValue;
  patchsetList(repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  formatChangeCreate(payload: NativeJsonValue): string;
  formatChangeShow(payload: NativeJsonValue): string;
  formatChangeRevert(payload: NativeJsonValue): string;
  formatChangeReplay(payload: NativeJsonValue): string;
  formatChangeClose(payload: NativeJsonValue): string;
  formatChangePublish(payload: NativeJsonValue): string;
  formatPatchsetPublish(payload: NativeJsonValue): string;
  formatPatchsetList(payload: NativeJsonValue): string;
  formatPatchsetShow(payload: NativeJsonValue): string;
  formatPatchsetSelect(payload: NativeJsonValue): string;
  formatPatchsetCiStatus(payload: NativeJsonValue): string;
  formatPatchsetRerunCi(payload: NativeJsonValue): string;
  projectChangeList(payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  formatChangeList(payload: NativeJsonValue, options?: ChangeListOutputOptions | null): string;
  taskWorkflowChangeList(root: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowChangeShow(root: string, changeId: string, options?: ChangeQueryOptions | null): NativeJsonValue;
  taskWorkflowChangeRevert(root: string, changeId: string, options?: ChangeMutationOptions | null): NativeJsonValue;
  taskWorkflowChangeReplay(root: string, changeId: string, onto: string, options?: ChangeMutationOptions | null): NativeJsonValue;
  taskWorkflowChangeClose(root: string, changeId: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowChangePublish(root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetPublish(root: string, changeId: string, summary: string, options?: PatchsetPublishOptions | null): NativeJsonValue;
  taskWorkflowPatchsetList(root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetShow(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetSelect(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetPublishExplicit(root: string, changeId: string, baseSnapshotId: string, revisionSnapshotId: string, summary: string, options?: PatchsetPublishExplicitOptions | null): NativeJsonValue;
  taskWorkflowPatchsetCiStatus(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetRerunCi(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowReviewTeamApprove(root: string, changeId: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  taskWorkflowReviewShow(root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowReviewRequest(root: string, changeId: string, reviewerGroups: string[], options?: ReviewRequestOptions | null): NativeJsonValue;
  taskWorkflowReviewTaskApprove(root: string, changeId: string, patchsetId: string, message: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowReviewRecord(root: string, changeId: string, action: string, options?: ReviewRecordOptions | null): NativeJsonValue;
  taskWorkflowReviewCodeSubmit(root: string, changeId: string, patchsetId: string, message: string, options?: ReviewCodeSubmitOptions | null): NativeJsonValue;
  taskWorkflowReviewCodeTemplate(options?: ReviewCodeTemplateOptions | null): NativeJsonValue;
  taskWorkflowAttestPut(root: string, options?: AttestationPutOptions | null): NativeJsonValue;
  taskWorkflowAttestShow(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPolicyEval(root: string, patchsetId: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowPolicyShow(root: string, patchsetId: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowPolicyWaive(root: string, patchsetId: string, ruleName: string, reason: string, options?: PolicyWaiveOptions | null): NativeJsonValue;
  repositoryShow(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  repositoryJobs(repoRoot: string, options?: RepositoryJobsOptions | null): NativeJsonValue;
  repositoryCiCapabilities(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  repositoryRetire(repoRoot: string, remoteName?: string | null, abort?: boolean | null): NativeJsonValue;
  repositoryRestore(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  formatRepositoryShow(payload: NativeJsonValue, remoteName?: string | null): string;
  formatRepositoryJobs(payload: NativeJsonValue, options?: RepositoryJobsOptions | null): string;
  formatRepositoryCiCapabilities(payload: NativeJsonValue, remoteName?: string | null): string;
  formatRepositoryRetire(payload: NativeJsonValue): string;
  formatRepositoryRestore(payload: NativeJsonValue): string;
  taskWorkflowLineList(repoRoot: string, options?: LineListOptions | null): NativeJsonValue;
  lineInventory(repoRoot: string, options?: LineListOptions | null): LineInventory;
  taskWorkflowLineCreate(repoRoot: string, name: string, options?: LineCreateOptions | null): NativeJsonValue;
  taskWorkflowLineSwitch(repoRoot: string, name: string, options?: LineSwitchOptions | null): NativeJsonValue;
  taskWorkflowLineShow(repoRoot: string, name?: string | null): NativeJsonValue;
  lineShow(repoRoot: string, name?: string | null, remoteName?: string | null): NativeJsonValue;
  taskWorkflowLineArchive(repoRoot: string, name: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowLineRename(repoRoot: string, oldName: string, newName: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowLineDelete(repoRoot: string, name: string, options?: LineDeleteOptions | null): NativeJsonValue;
  taskWorkflowLineSetHead(repoRoot: string, name: string, snapshotId?: string | null): NativeJsonValue;
  lineMerge(repoRoot: string, options?: LineMergeOptions | null): NativeJsonValue;
  taskWorkflowLineCleanup(repoRoot: string, options?: LineCleanupOptions | null): NativeJsonValue;
  formatLineList(inventory: LineInventory, options?: LineListOptions | null): string;
  formatLineCreate(payload: NativeJsonValue): string;
  formatLineSwitch(payload: NativeJsonValue): string;
  formatLineShow(payload: NativeJsonValue): string;
  formatLineArchive(payload: NativeJsonValue): string;
  formatLineRename(payload: NativeJsonValue): string;
  formatLineDelete(payload: NativeJsonValue): string;
  formatLineMerge(payload: NativeJsonValue): string;
  formatLineCleanup(payload: NativeJsonValue, idleFor: string, options?: LineCleanupOutputOptions | null): string;
  bindingInfoJson(): string;
  projectTaskList(payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  formatTaskList(payload: NativeJsonValue, options?: TaskListOutputOptions | null): string;
  formatTaskShow(payload: NativeJsonValue): string;
  projectSnapshotList(payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  formatSnapshotList(payload: NativeJsonValue, all?: boolean | null): string;
  inspectSnapshot(repoRoot: string, selector: string, includeDelta?: boolean | null): SnapshotInspection;
  formatSnapshotShow(inspection: SnapshotInspection, files?: boolean | null): string;
  snapshotAncestry(repoRoot: string, selector: string, options?: SnapshotAncestryOptions | null): NativeJsonValue;
  snapshotIsAncestor(repoRoot: string, older: string, newer: string): WorkflowPhaseResult;
  snapshotMergeBase(repoRoot: string, left: string, right: string, all?: boolean | null): WorkflowPhaseResult;
  formatSnapshotAncestry(payload: NativeJsonValue, all?: boolean | null): string;
  formatSnapshotIsAncestor(payload: NativeJsonValue): string;
  formatSnapshotMergeBase(payload: NativeJsonValue): string;
  formatSnapshotQueryError(message: string): string;
  snapshotQueryErrorExitCode(): number;
  formatInitializeRepository(payload: NativeJsonValue): string;
  formatConfigurationShow(payload: NativeJsonValue): string;
  formatConfiguration(payload: NativeJsonValue, requestPayload: NativeJsonValue): string;
  unsetConfiguration(repoRoot: string, key: string): NativeJsonValue;
  formatConfigurationUnset(payload: NativeJsonValue, key: string): string;
  formatRemoteAdd(payload: NativeJsonValue): string;
  formatRemoteList(payload: NativeJsonValue): string;
  queueSummary(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  formatQueueSummary(payload: NativeJsonValue): string;
  doctorMemoryRoot(repoRoot: string): NativeJsonValue;
  doctorPlanAuthority(repoRoot: string): NativeJsonValue;
  formatDoctorMemoryRoot(payload: NativeJsonValue): string;
  formatDoctorRuntimeRoot(payload: NativeJsonValue): string;
  formatDoctorPlanAuthority(payload: NativeJsonValue): string;
  repositoryAuthorityRoot(repoRoot: string): string;
  formatTaskQuick(payload: NativeJsonValue): string;
  taskAudit(repoRoot: string, taskId: string, options?: ScopeOptions | null): NativeJsonValue;
  formatTaskAudit(payload: NativeJsonValue, taskId: string): string;
  taskAbandon(repoRoot: string, taskId: string, options?: ScopeOptions | null): WorkflowPhaseResult;
  formatTaskAbandon(payload: NativeJsonValue, taskId: string): string;
  formatPlanList(payload: NativeJsonValue, options?: PlanListOutputOptions | null): string;
  formatPlanRevisions(payload: NativeJsonValue, planId: string, options?: PlanListOutputOptions | null): string;
  formatPlanShow(payload: NativeJsonValue): string;
  formatPlanItems(payload: NativeJsonValue): string;
  formatPlanInspect(payload: NativeJsonValue): string;
  formatPlanCandidates(payload: NativeJsonValue): string;
  formatPlanSync(payload: NativeJsonValue): string;
  workflowReady(repoRoot: string, taskId: string, options?: WorkflowReadyOptions | null): WorkflowPhaseResult;
  workflowFinish(repoRoot: string, taskId: string, options?: WorkflowFinishOptions | null): WorkflowPhaseResult;
  workflowReconcile(repoRoot: string, options?: WorkflowReconcileOptions | null): NativeJsonValue;
  workflowGuide(topic?: string | null): NativeJsonValue;
  projectWorkflowReady(repoRoot: string, payload: NativeJsonValue, taskId: string, options?: WorkflowOutputOptions | null): NativeJsonValue;
  projectWorkflowFinish(repoRoot: string, payload: NativeJsonValue, taskId: string, options?: WorkflowOutputOptions | null): NativeJsonValue;
  formatWorkflowReady(payload: NativeJsonValue, taskId: string): string;
  formatWorkflowFinish(payload: NativeJsonValue, taskId: string): string;
  formatWorkflowReconcile(payload: NativeJsonValue): string;
  formatWorkflowGuide(payload: NativeJsonValue): string;
  planList(repoRoot: string, options?: ScopeOptions | null): NativeJsonValue[];
  planShow(repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  planRevisions(repoRoot: string, planId: string, options?: ScopeOptions | null): NativeJsonValue[];
  planItems(repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  planInspect(repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  planCandidates(repoRoot: string, options?: PlanCandidatesOptions | null): NativeJsonValue;
  planSync(repoRoot: string, target: string, options?: PlanSyncOptions | null): NativeJsonValue;
  planSyncError(payload: NativeJsonValue): string | null;
  formatStatus(payload: NativeJsonValue): string;
  formatDomainError(message: string): string;
  domainErrorExitCode(): number;
  decodeJson(text: string): NativeJsonValue;
  initializeRepository(repoRoot: string, options?: InitOptions | null): NativeJsonValue;
  configureRepository(repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  configurationError(payload: NativeJsonValue): string | null;
  taskStart(repoRoot: string, request: TaskStartRequest): NativeJsonValue;
  taskQuick(repoRoot: string, request: TaskQuickRequest): NativeJsonValue;
  taskFinish(repoRoot: string, taskId: string, options?: TaskFinishOptions | null): NativeJsonValue;
  taskFinishExitCode(payload: NativeJsonValue): number;
  snapshotCapture(repoRoot: string, reference: string, options?: SnapshotCaptureOptions | null): NativeJsonValue;
  projectTaskStart(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  formatTaskStart(payload: NativeJsonValue): string;
  projectTaskFinish(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  formatTaskFinish(payload: NativeJsonValue, requested: string): string;
  projectSnapshotCapture(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  formatSnapshotCapture(payload: NativeJsonValue): string;
  projectStatus(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  encodeJson(value: NativeJsonValue, pretty?: boolean | null): string;
  planPortsNormalizePlanStoreReadRequest(payloadJson: string): NativeJsonValue;
  validatePlanningSessionJoinPayload(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanRemoteTransport(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanRemoteRequest(payloadJson: string): NativeJsonValue;
  planPortsNormalizeArtifactResolverRequest(payloadJson: string): NativeJsonValue;
  planPortsNormalizeArtifactPublishRequest(payloadJson: string): NativeJsonValue;
  planPortsNormalizeLinkedTaskLookupPayload(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanConfigRuntimeFacts(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanConnectionManagerStats(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeSelectionRequest(payloadJson: string): NativeJsonValue;
  planConfigRuntimeBuildSelectionFacts(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeSelectionFacts(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeCompatibility(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeReadiness(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeDoctor(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeRequest(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeBackendIdentity(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeCompatibility(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeReadiness(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeDoctor(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildBackendIdentityFacts(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildStorageReadinessFacts(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildCompatibilityStatus(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildReadinessStatus(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildDoctorFacts(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeListRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildList(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeShowRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildShow(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeRevisionsRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildRevisions(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeItemsRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildItems(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeCandidatesRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildCandidates(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeInspectRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildInspect(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeSyncRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildSync(payloadJson: string): NativeJsonValue;
  planCommandNormalizeListRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildList(payloadJson: string): NativeJsonValue;
  planCommandNormalizeShowRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildShow(payloadJson: string): NativeJsonValue;
  planCommandNormalizeRevisionsRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildRevisions(payloadJson: string): NativeJsonValue;
  planCommandNormalizeItemsRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildItems(payloadJson: string): NativeJsonValue;
  planCommandNormalizeCandidatesRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildCandidates(payloadJson: string): NativeJsonValue;
  planCommandNormalizeInspectRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildInspect(payloadJson: string): NativeJsonValue;
  planCommandNormalizeSyncRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildSync(payloadJson: string): NativeJsonValue;
  planCommandExecuteList(payloadJson: string): NativeJsonValue;
  planCommandExecuteShow(payloadJson: string): NativeJsonValue;
  planCommandExecuteRevisions(payloadJson: string): NativeJsonValue;
  planCommandExecuteItems(payloadJson: string): NativeJsonValue;
  planCommandExecuteCandidates(payloadJson: string): NativeJsonValue;
  planCommandExecuteInspect(payloadJson: string): NativeJsonValue;
  planCommandExecuteSync(payloadJson: string): NativeJsonValue;
  planProvenanceNormalizeRevisionProvenance(payloadJson: string): NativeJsonValue;
  planProvenanceBuildRevisionProvenance(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeTimestampRequest(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeTimestamp(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildTimestamp(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeSequenceIdentityRequest(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeSequenceIdentity(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildSequenceIdentity(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeWorkflowIdRequest(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeWorkflowId(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildWorkflowId(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeTemporalOrdering(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildTemporalOrdering(payloadJson: string): NativeJsonValue;
  taskWorkflowInit(repoRoot: string, options?: InitOptions | null): NativeJsonValue;
  taskWorkflowConfigShow(repoRoot: string): NativeJsonValue;
  taskWorkflowConfigSet(repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  taskWorkflowStatus(repoRoot: string): NativeJsonValue;
  taskWorkflowRemoteAdd(repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  taskWorkflowRemoteList(repoRoot: string): NativeJsonValue;
  taskWorkflowDoctorRuntimeRoot(repoRoot: string): NativeJsonValue;
  taskWorkflowDoctorPlanAuthority(): NativeJsonValue;
  taskWorkflowTaskStart(repoRoot: string, title: string, intent: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowTaskList(repoRoot: string, options?: TaskListOptions | null): NativeJsonValue[];
  taskWorkflowTaskShow(repoRoot: string, taskId: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowSnapshotCreate(repoRoot: string, message?: string | null): NativeJsonValue;
  taskWorkflowSnapshotList(repoRoot: string): NativeJsonValue;
  taskWorkflowSnapshotShow(repoRoot: string, snapshotId: string): NativeJsonValue;
  /** @deprecated Throws: Chat integrations are retired in AIT 1.2.0. */
  agentWorkerCapabilitiesJson(): string;
  /** @deprecated Throws: Chat integrations are retired in AIT 1.2.0. */
  agentManagementJson(requestJson: string): string;
  /** @deprecated Throws: Chat integrations are retired in AIT 1.2.0. */
  agentWorkerTransactionJson(requestJson: string): string;
}

export interface LanguageBindingInfo extends JsonObject {
  contract: "ait.language.binding.v1";
  version: string;
  runtime_authority: "rust";
  python_binding: "pyo3";
  node_binding: "napi";
  process_transport_allowed: false;
  supported_surfaces: ["ait-core"];
}

export class NativeRuntime {
  readonly CODE_REVIEW_SUMMARY_TEMPLATE: string;
  readonly CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE: string;
  readonly CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND: string;
  readonly DEFAULT_ID_NAMESPACE_PREFIX: string;
  readonly LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX: string;
  readonly REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX: string;
  readonly RUNTIME_DATA_ENV: string;
  readonly LEGACY_SERVER_DATA_ENV: string;
  readonly CONTRACT_VERSION: string;
  readonly PLAN_CONTRACT_VERSION: string;
  readonly TARGET_TRIPLE: string;
  readonly DEFAULT_REQUEST_TIMEOUT_SECONDS: 30;
  readonly LONG_RUNNING_REQUEST_TIMEOUT_SECONDS: 1800;
  remoteRecoverHead(repoRoot: string, options?: RemoteHeadRecoveryOptions | null): NativeJsonValue;
  formatRemoteRecoverHead(payload: NativeJsonValue): string;
  parseSnapshotAuthoringReference(reference: string): string;
  workflowIdFamilies(): string[];
  workflowTaskChangeOriginNamespacePrefixes(): string[];
  policyRequirementFlags(): string[];
  call(method: "remoteRecoverHead", repoRoot: string, options?: RemoteHeadRecoveryOptions | null): NativeJsonValue;
  call(method: "formatRemoteRecoverHead", payload: NativeJsonValue): string;
  call(method: "parseSnapshotAuthoringReference", reference: string): string;
  call(method: "workflowIdFamilies"): string[];
  call(method: "workflowTaskChangeOriginNamespacePrefixes"): string[];
  call(method: "policyRequirementFlags"): string[];

  externalStatus(repoRoot: string): NativeJsonValue;
  externalDoctor(repoRoot: string, failOnBlocking?: boolean | null): WorkflowPhaseResult;
  externalUpdate(repoRoot: string, options?: ExternalUpdateOptions | null): NativeJsonValue;
  externalLink(repoRoot: string, name: string, path: string): NativeJsonValue;
  externalUnlink(repoRoot: string, name: string): NativeJsonValue;
  teamContacts(repoRoot: string): WorkflowPhaseResult;
  teamSessionEnsure(repoRoot: string, agent: number | bigint, role?: number | bigint | null): WorkflowPhaseResult;
  teamSessionAcknowledge(repoRoot: string, activation: string): WorkflowPhaseResult;
  formatTeamContacts(payload: NativeJsonValue): string;
  formatTeamSessionEnsure(payload: NativeJsonValue, agent: number | bigint): string;
  formatTeamSessionAcknowledge(payload: NativeJsonValue): string;
  binaryDbUpgradeU64Seconds(repoRoot: string, generationRoot: string, sourceTimeWidth: string, activate?: boolean | null): NativeJsonValue;
  binaryDbAgentPool50(repoRoot: string, generationRoot: string, activate?: boolean | null): NativeJsonValue;
  formatBinaryDbUpgradeU64Seconds(payload: NativeJsonValue): string;
  formatBinaryDbAgentPool50(payload: NativeJsonValue): string;
  formatExternalStatus(payload: NativeJsonValue): string;
  formatExternalDoctor(payload: NativeJsonValue): string;
  formatExternalUpdate(payload: NativeJsonValue): string;
  formatExternalLink(payload: NativeJsonValue): string;
  formatExternalUnlink(payload: NativeJsonValue): string;
  call(method: "externalStatus", repoRoot: string): NativeJsonValue;
  call(method: "externalDoctor", repoRoot: string, failOnBlocking?: boolean | null): WorkflowPhaseResult;
  call(method: "externalUpdate", repoRoot: string, options?: ExternalUpdateOptions | null): NativeJsonValue;
  call(method: "externalLink", repoRoot: string, name: string, path: string): NativeJsonValue;
  call(method: "externalUnlink", repoRoot: string, name: string): NativeJsonValue;
  call(method: "teamContacts", repoRoot: string): WorkflowPhaseResult;
  call(method: "teamSessionEnsure", repoRoot: string, agent: number | bigint, role?: number | bigint | null): WorkflowPhaseResult;
  call(method: "teamSessionAcknowledge", repoRoot: string, activation: string): WorkflowPhaseResult;
  call(method: "formatTeamContacts", payload: NativeJsonValue): string;
  call(method: "formatTeamSessionEnsure", payload: NativeJsonValue, agent: number | bigint): string;
  call(method: "formatTeamSessionAcknowledge", payload: NativeJsonValue): string;
  call(method: "binaryDbUpgradeU64Seconds", repoRoot: string, generationRoot: string, sourceTimeWidth: string, activate?: boolean | null): NativeJsonValue;
  call(method: "binaryDbAgentPool50", repoRoot: string, generationRoot: string, activate?: boolean | null): NativeJsonValue;
  call(method: "formatBinaryDbUpgradeU64Seconds", payload: NativeJsonValue): string;
  call(method: "formatBinaryDbAgentPool50", payload: NativeJsonValue): string;
  call(method: "formatExternalStatus", payload: NativeJsonValue): string;
  call(method: "formatExternalDoctor", payload: NativeJsonValue): string;
  call(method: "formatExternalUpdate", payload: NativeJsonValue): string;
  call(method: "formatExternalLink", payload: NativeJsonValue): string;
  call(method: "formatExternalUnlink", payload: NativeJsonValue): string;

  releaseAdapterCheck(repoRoot: string, version: string, options?: ReleaseAdapterOptions | null): NativeJsonValue;
  releaseAdapterBuild(repoRoot: string, version: string, options?: ReleaseAdapterOptions | null): NativeJsonValue;
  releaseCandidateCreate(repoRoot: string, version: string, options?: ReleaseCandidateOptions | null): NativeJsonValue;
  releaseCheck(repoRoot: string, releaseId: string, options?: ReleaseCheckOptions | null): NativeJsonValue;
  releaseBuild(repoRoot: string, releaseId: string, options?: ReleaseBuildOptions | null): NativeJsonValue;
  releaseNativeSource(repoRoot: string, request: ReleaseNativeSourceInput): NativeJsonValue;
  releaseNativeBundle(repoRoot: string, releaseId: string, nativeMatrixDir: string): NativeJsonValue;
  releasePackage(repoRoot: string, releaseId: string, channel: string, publicSourceRoot?: string | null): NativeJsonValue;
  releaseFormula(repoRoot: string, releaseId: string, name: string, pythonFormula?: string | null): NativeJsonValue;
  releaseShow(repoRoot: string, releaseId: string, options?: ReleaseShowOptions | null): NativeJsonValue;
  releasePublish(repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;
  releasePromote(repoRoot: string, releaseId: string, channel: string, publicSourceRoot?: string | null): NativeJsonValue;
  formatRelease(payload: NativeJsonValue): string;
  call(method: "releaseAdapterCheck", repoRoot: string, version: string, options?: ReleaseAdapterOptions | null): NativeJsonValue;
  call(method: "releaseAdapterBuild", repoRoot: string, version: string, options?: ReleaseAdapterOptions | null): NativeJsonValue;
  call(method: "releaseCandidateCreate", repoRoot: string, version: string, options?: ReleaseCandidateOptions | null): NativeJsonValue;
  call(method: "releaseCheck", repoRoot: string, releaseId: string, options?: ReleaseCheckOptions | null): NativeJsonValue;
  call(method: "releaseBuild", repoRoot: string, releaseId: string, options?: ReleaseBuildOptions | null): NativeJsonValue;
  call(method: "releaseNativeSource", repoRoot: string, request: ReleaseNativeSourceInput): NativeJsonValue;
  call(method: "releaseNativeBundle", repoRoot: string, releaseId: string, nativeMatrixDir: string): NativeJsonValue;
  call(method: "releasePackage", repoRoot: string, releaseId: string, channel: string, publicSourceRoot?: string | null): NativeJsonValue;
  call(method: "releaseFormula", repoRoot: string, releaseId: string, name: string, pythonFormula?: string | null): NativeJsonValue;
  call(method: "releaseShow", repoRoot: string, releaseId: string, options?: ReleaseShowOptions | null): NativeJsonValue;
  call(method: "releasePublish", repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;
  call(method: "releasePromote", repoRoot: string, releaseId: string, channel: string, publicSourceRoot?: string | null): NativeJsonValue;
  call(method: "formatRelease", payload: NativeJsonValue): string;

  createWorkflowRelease(repoRoot: string, input: CreateWorkflowReleaseInput): NativeJsonValue;
  listWorkflowReleases(repoRoot: string): NativeJsonValue;
  getWorkflowRelease(repoRoot: string, releaseId: string): NativeJsonValue;
  updateWorkflowRelease(repoRoot: string, releaseId: string, options?: UpdateWorkflowReleaseOptions | null): NativeJsonValue;
  taskWorkflowReleaseCandidateCreate(repoRoot: string, version: string, line: string, profile: string): NativeJsonValue;
  taskWorkflowReleaseCheck(repoRoot: string, releaseId: string, options?: ReleaseCheckSdkOptions | null): NativeJsonValue;
  taskWorkflowReleaseBuild(repoRoot: string, releaseId: string): NativeJsonValue;
  taskWorkflowReleaseFormula(repoRoot: string, releaseId: string, name: string): NativeJsonValue;
  taskWorkflowReleaseShow(repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowReleasePublish(repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;
  call(method: "createWorkflowRelease", repoRoot: string, input: CreateWorkflowReleaseInput): NativeJsonValue;
  call(method: "listWorkflowReleases", repoRoot: string): NativeJsonValue;
  call(method: "getWorkflowRelease", repoRoot: string, releaseId: string): NativeJsonValue;
  call(method: "updateWorkflowRelease", repoRoot: string, releaseId: string, options?: UpdateWorkflowReleaseOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReleaseCandidateCreate", repoRoot: string, version: string, line: string, profile: string): NativeJsonValue;
  call(method: "taskWorkflowReleaseCheck", repoRoot: string, releaseId: string, options?: ReleaseCheckSdkOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReleaseBuild", repoRoot: string, releaseId: string): NativeJsonValue;
  call(method: "taskWorkflowReleaseFormula", repoRoot: string, releaseId: string, name: string): NativeJsonValue;
  call(method: "taskWorkflowReleaseShow", repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;
  call(method: "taskWorkflowReleasePublish", repoRoot: string, releaseId: string, remoteName?: string | null): NativeJsonValue;

  projectWorkflowLandedReadModel(context: WorkflowProjectionContext, state: NativeJsonValue): NativeJsonValue;
  projectWorkflowLandFullReadModel(context: WorkflowProjectionContext, state: NativeJsonValue, applyOwnedContinuation: boolean): NativeJsonValue;
  projectWorkflowReadyReadModel(context: WorkflowProjectionContext, state: NativeJsonValue, ignoreWorkspaceAuthoring: boolean, patchsetIsAuthoritative: boolean, applyOwnedContinuation: boolean): NativeJsonValue;
  call(method: "projectWorkflowLandedReadModel", context: WorkflowProjectionContext, state: NativeJsonValue): NativeJsonValue;
  call(method: "projectWorkflowLandFullReadModel", context: WorkflowProjectionContext, state: NativeJsonValue, applyOwnedContinuation: boolean): NativeJsonValue;
  call(method: "projectWorkflowReadyReadModel", context: WorkflowProjectionContext, state: NativeJsonValue, ignoreWorkspaceAuthoring: boolean, patchsetIsAuthoritative: boolean, applyOwnedContinuation: boolean): NativeJsonValue;

  buildLinkedTaskLookupPayload(taskLinksByItemRows?: NativeJsonValue | null, tasksByPlanRows?: NativeJsonValue | null): NativeJsonValue;
  buildLinkedChangeLookupPayload(changeLinksByTaskRows?: NativeJsonValue | null): NativeJsonValue;
  buildTaskTrackingTitlePayload(task: NativeJsonValue): NativeJsonValue;
  buildTaskTrackingMetadataPayload(task: NativeJsonValue, authorMode: string, trackingPolicy: string): NativeJsonValue;
  buildTaskAuditVerdictPayload(task: NativeJsonValue, changeRows: NativeJsonValue, targetLine: string): NativeJsonValue;
  workflowLandedFacts(state: NativeJsonValue): NativeJsonValue;
  workflowLandFullFacts(state: NativeJsonValue): NativeJsonValue;
  workflowReadyFacts(state: NativeJsonValue): NativeJsonValue;
  workflowLandPhaseFacts(state: NativeJsonValue, readyState: NativeJsonValue): NativeJsonValue;
  projectWorkflowLandPhaseReadModel(state: NativeJsonValue, readyState: NativeJsonValue): NativeJsonValue;
  workflowRemoteMutationReceipt(input: RemoteMutationReceiptInput): NativeJsonValue;
  workflowRemoteActionMutationReceipts(code: string, result: NativeJsonValue): NativeJsonValue[];
  workflowApplyPhasePayload(phase: string, code: string, options?: ApplyPhaseOptions | null): NativeJsonValue;
  workflowApplyPhaseSummary(applyPhase: NativeJsonValue): string | null;
  workflowMutationReceiptSummary(receipt: NativeJsonValue): string;
  workflowAppliedActionSummary(action: NativeJsonValue): string;
  workflowReadyPayloadDirect(repoRoot: string, changeId: string, remoteName?: string | null): NativeJsonValue;
  workflowLandPayloadDirect(repoRoot: string, changeId: string, remoteName?: string | null): NativeJsonValue;
  workflowReadyApplyDirect(repoRoot: string, changeId: string, options?: WorkflowReadyDirectOptions | null): NativeJsonValue;
  workflowLandApplyDirect(repoRoot: string, changeId: string, options?: WorkflowLandDirectOptions | null): NativeJsonValue;
  call(method: "buildLinkedTaskLookupPayload", taskLinksByItemRows?: NativeJsonValue | null, tasksByPlanRows?: NativeJsonValue | null): NativeJsonValue;
  call(method: "buildLinkedChangeLookupPayload", changeLinksByTaskRows?: NativeJsonValue | null): NativeJsonValue;
  call(method: "buildTaskTrackingTitlePayload", task: NativeJsonValue): NativeJsonValue;
  call(method: "buildTaskTrackingMetadataPayload", task: NativeJsonValue, authorMode: string, trackingPolicy: string): NativeJsonValue;
  call(method: "buildTaskAuditVerdictPayload", task: NativeJsonValue, changeRows: NativeJsonValue, targetLine: string): NativeJsonValue;
  call(method: "workflowLandedFacts", state: NativeJsonValue): NativeJsonValue;
  call(method: "workflowLandFullFacts", state: NativeJsonValue): NativeJsonValue;
  call(method: "workflowReadyFacts", state: NativeJsonValue): NativeJsonValue;
  call(method: "workflowLandPhaseFacts", state: NativeJsonValue, readyState: NativeJsonValue): NativeJsonValue;
  call(method: "projectWorkflowLandPhaseReadModel", state: NativeJsonValue, readyState: NativeJsonValue): NativeJsonValue;
  call(method: "workflowRemoteMutationReceipt", input: RemoteMutationReceiptInput): NativeJsonValue;
  call(method: "workflowRemoteActionMutationReceipts", code: string, result: NativeJsonValue): NativeJsonValue[];
  call(method: "workflowApplyPhasePayload", phase: string, code: string, options?: ApplyPhaseOptions | null): NativeJsonValue;
  call(method: "workflowApplyPhaseSummary", applyPhase: NativeJsonValue): string | null;
  call(method: "workflowMutationReceiptSummary", receipt: NativeJsonValue): string;
  call(method: "workflowAppliedActionSummary", action: NativeJsonValue): string;
  call(method: "workflowReadyPayloadDirect", repoRoot: string, changeId: string, remoteName?: string | null): NativeJsonValue;
  call(method: "workflowLandPayloadDirect", repoRoot: string, changeId: string, remoteName?: string | null): NativeJsonValue;
  call(method: "workflowReadyApplyDirect", repoRoot: string, changeId: string, options?: WorkflowReadyDirectOptions | null): NativeJsonValue;
  call(method: "workflowLandApplyDirect", repoRoot: string, changeId: string, options?: WorkflowLandDirectOptions | null): NativeJsonValue;

  taskWorkflowTimestampFacts(now?: string | null): NativeJsonValue;
  taskWorkflowSequenceIdentityFacts(family: string, number: number | bigint, options?: SequenceFactsOptions | null): NativeJsonValue;
  taskWorkflowWorkflowIdFacts(family: string, options?: WorkflowIdFactsOptions | null): NativeJsonValue;
  taskWorkflowRuntimeSelectionFacts(overrides?: NativeJsonValue | null): NativeJsonValue;
  taskWorkflowReadUtf8TextFile(pathValue: string): string;
  taskWorkflowReadJsonFile(pathValue: string): NativeJsonValue;
  taskWorkflowReadBinaryFile(pathValue: string): Buffer;
  taskWorkflowResolveRepoArtifactPath(repoRoot: string, pathValue: string, allowMissing?: boolean | null): ResolvedRepoArtifactPath;
  taskRemoteChangeLineagePayload(baseLine: string, lineRow?: NativeJsonValue | null): NativeJsonValue;
  taskWorkflowResolveTaskWorktreeLocation(repoRoot: string, worktreeName: string, debugProbeOverride?: NativeJsonValue | null): NativeJsonValue;
  taskWorkflowResolveMainSeedMirrorLocation(repoRoot: string, seedName: string): NativeJsonValue;
  taskWorkflowEnsureMainSeedMirror(repoRoot: string, options?: EnsureSeedMirrorOptions | null): NativeJsonValue;
  taskWorkflowEnsureStatusManifest(repoRoot: string, snapshotId: string): NativeJsonValue;
  taskWorkflowQueueSummary(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowTaskAudit(repoRoot: string, taskId: string, scope?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowTaskAbandon(repoRoot: string, taskId: string, scope?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowTaskLand(repoRoot: string, taskOrChangeId: string, scope?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowLandSubmit(repoRoot: string, changeId: string, options: LandSubmitOptions): NativeJsonValue;
  taskWorkflowLandShow(repoRoot: string, submissionId: string, options?: LandReceiptOptions | null): NativeJsonValue;
  taskWorkflowLandRetry(repoRoot: string, submissionId: string, options?: LandRetryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowTimestampFacts", now?: string | null): NativeJsonValue;
  call(method: "taskWorkflowSequenceIdentityFacts", family: string, number: number | bigint, options?: SequenceFactsOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorkflowIdFacts", family: string, options?: WorkflowIdFactsOptions | null): NativeJsonValue;
  call(method: "taskWorkflowRuntimeSelectionFacts", overrides?: NativeJsonValue | null): NativeJsonValue;
  call(method: "taskWorkflowReadUtf8TextFile", pathValue: string): string;
  call(method: "taskWorkflowReadJsonFile", pathValue: string): NativeJsonValue;
  call(method: "taskWorkflowReadBinaryFile", pathValue: string): Buffer;
  call(method: "taskWorkflowResolveRepoArtifactPath", repoRoot: string, pathValue: string, allowMissing?: boolean | null): ResolvedRepoArtifactPath;
  call(method: "taskRemoteChangeLineagePayload", baseLine: string, lineRow?: NativeJsonValue | null): NativeJsonValue;
  call(method: "taskWorkflowResolveTaskWorktreeLocation", repoRoot: string, worktreeName: string, debugProbeOverride?: NativeJsonValue | null): NativeJsonValue;
  call(method: "taskWorkflowResolveMainSeedMirrorLocation", repoRoot: string, seedName: string): NativeJsonValue;
  call(method: "taskWorkflowEnsureMainSeedMirror", repoRoot: string, options?: EnsureSeedMirrorOptions | null): NativeJsonValue;
  call(method: "taskWorkflowEnsureStatusManifest", repoRoot: string, snapshotId: string): NativeJsonValue;
  call(method: "taskWorkflowQueueSummary", repoRoot: string, remoteName?: string | null): NativeJsonValue;
  call(method: "taskWorkflowTaskAudit", repoRoot: string, taskId: string, scope?: ScopeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowTaskAbandon", repoRoot: string, taskId: string, scope?: ScopeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowTaskLand", repoRoot: string, taskOrChangeId: string, scope?: ScopeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowLandSubmit", repoRoot: string, changeId: string, options: LandSubmitOptions): NativeJsonValue;
  call(method: "taskWorkflowLandShow", repoRoot: string, submissionId: string, options?: LandReceiptOptions | null): NativeJsonValue;
  call(method: "taskWorkflowLandRetry", repoRoot: string, submissionId: string, options?: LandRetryOptions | null): NativeJsonValue;

  extractCodexUsageJsonl(usageJsonlPath: string, usageScope: string): NativeJsonValue;
  extractCodexUsageBundleJsonl(usageJsonlPaths: string[], options?: UsageBundleOptions | null): NativeJsonValue;
  projectRepoRetireRuntimeBlockers(payload: NativeJsonValue): NativeJsonValue;
  languageBindingInfo(): LanguageBindingInfo;
  transportEnvelopeIrVersion(): string;
  transportEnvelopeSchema(): NativeJsonValue;
  runtimeBindingStateIrVersion(): string;
  runtimeBindingStateSchema(): NativeJsonValue;
  runtimeBindingStateDefaultPayload(): NativeJsonValue;
  runtimeBindingStateNormalizeDocument(payload: NativeJsonValue): NativeJsonValue;
  workerManifestIrVersion(): string;
  workerManifestSchema(): NativeJsonValue;
  workerManifestDefaultConfig(): NativeJsonValue;
  workerManifestNormalizeDocument(payload: NativeJsonValue, path?: string | null): NativeJsonValue;
  workerManifestSelectTelegramWorker(config: NativeJsonValue, requestedName?: string | null): NativeJsonValue;
  transportEnvelopeBuildBindingMetadata(input: TransportBindingMetadataInput): NativeJsonValue;
  transportEnvelopeBuildEventEnvelope(input: TransportEventInput): NativeJsonValue;
  transportEnvelopeBuildReplyEnvelope(input: TransportReplyInput): NativeJsonValue;
  transportEnvelopeCompactEventEnvelope(envelope: NativeJsonValue): NativeJsonValue;
  transportEnvelopeCompactReplyEnvelope(envelope: NativeJsonValue): NativeJsonValue;
  call(method: "extractCodexUsageJsonl", usageJsonlPath: string, usageScope: string): NativeJsonValue;
  call(method: "extractCodexUsageBundleJsonl", usageJsonlPaths: string[], options?: UsageBundleOptions | null): NativeJsonValue;
  call(method: "projectRepoRetireRuntimeBlockers", payload: NativeJsonValue): NativeJsonValue;
  call(method: "languageBindingInfo"): LanguageBindingInfo;
  call(method: "transportEnvelopeIrVersion"): string;
  call(method: "transportEnvelopeSchema"): NativeJsonValue;
  call(method: "runtimeBindingStateIrVersion"): string;
  call(method: "runtimeBindingStateSchema"): NativeJsonValue;
  call(method: "runtimeBindingStateDefaultPayload"): NativeJsonValue;
  call(method: "runtimeBindingStateNormalizeDocument", payload: NativeJsonValue): NativeJsonValue;
  call(method: "workerManifestIrVersion"): string;
  call(method: "workerManifestSchema"): NativeJsonValue;
  call(method: "workerManifestDefaultConfig"): NativeJsonValue;
  call(method: "workerManifestNormalizeDocument", payload: NativeJsonValue, path?: string | null): NativeJsonValue;
  call(method: "workerManifestSelectTelegramWorker", config: NativeJsonValue, requestedName?: string | null): NativeJsonValue;
  call(method: "transportEnvelopeBuildBindingMetadata", input: TransportBindingMetadataInput): NativeJsonValue;
  call(method: "transportEnvelopeBuildEventEnvelope", input: TransportEventInput): NativeJsonValue;
  call(method: "transportEnvelopeBuildReplyEnvelope", input: TransportReplyInput): NativeJsonValue;
  call(method: "transportEnvelopeCompactEventEnvelope", envelope: NativeJsonValue): NativeJsonValue;
  call(method: "transportEnvelopeCompactReplyEnvelope", envelope: NativeJsonValue): NativeJsonValue;

  joinPath(baseUrl: string, path: string): string;
  buildQueryString(params: Record<string, string>): string | null;
  normalizeRemoteText(value?: string | null): string | null;
  buildListPlansRequest(baseUrl: string, repoName: string, artifactPath?: string | null): PlanHttpRequest;
  buildGetPlanRequest(baseUrl: string, planId: string): PlanHttpRequest;
  buildListPlanRevisionsRequest(baseUrl: string, planId: string): PlanHttpRequest;
  buildGetPlanRevisionRequest(baseUrl: string, planId: string, planRevisionId: string): PlanHttpRequest;
  buildCreatePlanRequest(baseUrl: string, input: CreatePlanInput): PlanHttpRequest;
  buildRevisePlanRequest(baseUrl: string, input: RevisePlanInput): PlanHttpRequest;
  buildUpdatePlanStatusRequest(baseUrl: string, planId: string, status: string): PlanHttpRequest;
  buildPutPlanRevisionArtifactsRequest(baseUrl: string, planId: string, planRevisionId: string, artifacts: NativeJsonValue[]): PlanHttpRequest;
  listPlans(baseUrl: string, repoName: string, artifactPath?: string | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  getPlan(baseUrl: string, planId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanRevisions(baseUrl: string, planId: string, options?: PlanHttpOptions | null): NativeJsonValue[];
  getPlanRevision(baseUrl: string, planId: string, planRevisionId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  resolveTaskPlanLinkage(baseUrl: string, repoName: string, linkage?: TaskPlanLinkageOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanIdsMatchingContains(baseUrl: string, repoName: string, containsTerms: string[], options?: PlanHttpOptions | null): NativeJsonValue[];
  createPlan(baseUrl: string, input: CreatePlanInput, options?: PlanHttpOptions | null): NativeJsonValue;
  revisePlan(baseUrl: string, input: RevisePlanInput, options?: PlanHttpOptions | null): NativeJsonValue;
  updatePlanStatus(baseUrl: string, planId: string, status: string, options?: PlanHttpOptions | null): NativeJsonValue;
  putPlanRevisionArtifacts(baseUrl: string, planId: string, planRevisionId: string, artifacts: NativeJsonValue[], options?: PlanHttpOptions | null): NativeJsonValue;
  createPlanningSession(baseUrl: string, planId: string, input?: CreatePlanningSessionOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanningSessions(baseUrl: string, planId: string, status?: string | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  getPlanningSession(baseUrl: string, planningSessionId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  appendPlanningSessionEvent(baseUrl: string, planningSessionId: string, eventType: string, payload?: NativeJsonValue | null, options?: PlanHttpOptions | null): NativeJsonValue;
  listPlanningSessionEvents(baseUrl: string, planningSessionId: string, input?: PlanningSessionEventsOptions | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  joinPlanningSession(baseUrl: string, planningSessionId: string, input?: JoinPlanningSessionOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  promotePlanningSession(baseUrl: string, planningSessionId: string, input: PromotePlanningSessionInput, options?: PlanHttpOptions | null): NativeJsonValue;
  closePlanningSession(baseUrl: string, planningSessionId: string, status?: string | null, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "joinPath", baseUrl: string, path: string): string;
  call(method: "buildQueryString", params: Record<string, string>): string | null;
  call(method: "normalizeRemoteText", value?: string | null): string | null;
  call(method: "buildListPlansRequest", baseUrl: string, repoName: string, artifactPath?: string | null): PlanHttpRequest;
  call(method: "buildGetPlanRequest", baseUrl: string, planId: string): PlanHttpRequest;
  call(method: "buildListPlanRevisionsRequest", baseUrl: string, planId: string): PlanHttpRequest;
  call(method: "buildGetPlanRevisionRequest", baseUrl: string, planId: string, planRevisionId: string): PlanHttpRequest;
  call(method: "buildCreatePlanRequest", baseUrl: string, input: CreatePlanInput): PlanHttpRequest;
  call(method: "buildRevisePlanRequest", baseUrl: string, input: RevisePlanInput): PlanHttpRequest;
  call(method: "buildUpdatePlanStatusRequest", baseUrl: string, planId: string, status: string): PlanHttpRequest;
  call(method: "buildPutPlanRevisionArtifactsRequest", baseUrl: string, planId: string, planRevisionId: string, artifacts: NativeJsonValue[]): PlanHttpRequest;
  call(method: "listPlans", baseUrl: string, repoName: string, artifactPath?: string | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  call(method: "getPlan", baseUrl: string, planId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "listPlanRevisions", baseUrl: string, planId: string, options?: PlanHttpOptions | null): NativeJsonValue[];
  call(method: "getPlanRevision", baseUrl: string, planId: string, planRevisionId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "resolveTaskPlanLinkage", baseUrl: string, repoName: string, linkage?: TaskPlanLinkageOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "listPlanIdsMatchingContains", baseUrl: string, repoName: string, containsTerms: string[], options?: PlanHttpOptions | null): NativeJsonValue[];
  call(method: "createPlan", baseUrl: string, input: CreatePlanInput, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "revisePlan", baseUrl: string, input: RevisePlanInput, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "updatePlanStatus", baseUrl: string, planId: string, status: string, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "putPlanRevisionArtifacts", baseUrl: string, planId: string, planRevisionId: string, artifacts: NativeJsonValue[], options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "createPlanningSession", baseUrl: string, planId: string, input?: CreatePlanningSessionOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "listPlanningSessions", baseUrl: string, planId: string, status?: string | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  call(method: "getPlanningSession", baseUrl: string, planningSessionId: string, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "appendPlanningSessionEvent", baseUrl: string, planningSessionId: string, eventType: string, payload?: NativeJsonValue | null, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "listPlanningSessionEvents", baseUrl: string, planningSessionId: string, input?: PlanningSessionEventsOptions | null, options?: PlanHttpOptions | null): NativeJsonValue[];
  call(method: "joinPlanningSession", baseUrl: string, planningSessionId: string, input?: JoinPlanningSessionOptions | null, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "promotePlanningSession", baseUrl: string, planningSessionId: string, input: PromotePlanningSessionInput, options?: PlanHttpOptions | null): NativeJsonValue;
  call(method: "closePlanningSession", baseUrl: string, planningSessionId: string, status?: string | null, options?: PlanHttpOptions | null): NativeJsonValue;

  planPackSubstrateBuildPackMembers(blobItems: NativeJsonValue, options?: BuildPackMembersOptions | null): NativeJsonValue;
  planPackSubstrateBuildTreePackMembers(treeRows: NativeJsonValue, treeEntryRows: NativeJsonValue): NativeJsonValue;
  planPackSubstrateBuildGitBinaryDelta(baseData: Buffer, targetData: Buffer): Buffer;
  planPackSubstrateBuildGitBinaryDeltaMember(entryName: string, blobId: string, baseBlobId: string, baseData: Buffer, targetData: Buffer, chainDepth: number | bigint): NativeJsonValue;
  planPackSubstrateApplyGitBinaryDelta(baseData: Buffer, deltaData: Buffer): Buffer;
  planPackSubstrateApplyPackDelta(baseData: Buffer, deltaData: Buffer, algorithm: string): Buffer;
  planPackSubstrateWritePackArchive(packPath: string, packId: string, createdAt: string, members: NativeJsonValue): NativeJsonValue;
  planPackSubstrateWriteTreePackArchive(packPath: string, packId: string, createdAt: string, members: NativeJsonValue): NativeJsonValue;
  planPackSubstrateReadPackIndex(packPath: string): NativeJsonValue;
  planPackSubstrateReadTreePackIndex(packPath: string): NativeJsonValue;
  planPackSubstratePackHasEntry(packPath: string, entryName: string): boolean;
  planPackSubstrateReadTreePackTree(packPath: string, treeId: string): NativeJsonValue;
  planPackSubstrateSummarizePackArchives(root: string, packRows: NativeJsonValue): NativeJsonValue;
  planPackSubstrateSummarizeTreePackArchives(root: string, packRows: NativeJsonValue): NativeJsonValue;
  planPackSubstrateTreePackManifestPath(packPath: string, entryName: string): string;
  planPackSubstrateReadTreePackTreeByOrdinal(packPath: string, entryOrdinal: number | bigint): NativeJsonValue;
  planPackSubstrateBuildStorageValidationSummary(options: StorageValidationOptions): NativeJsonValue;
  planPackSubstrateBuildTreeRecords(fileEntries: NativeJsonValue): NativeJsonValue;
  planPackSubstrateBuildSnapshotId(options: BuildSnapshotIdOptions): NativeJsonValue;
  call(method: "planPackSubstrateBuildPackMembers", blobItems: NativeJsonValue, options?: BuildPackMembersOptions | null): NativeJsonValue;
  call(method: "planPackSubstrateBuildTreePackMembers", treeRows: NativeJsonValue, treeEntryRows: NativeJsonValue): NativeJsonValue;
  call(method: "planPackSubstrateBuildGitBinaryDelta", baseData: Buffer, targetData: Buffer): Buffer;
  call(method: "planPackSubstrateBuildGitBinaryDeltaMember", entryName: string, blobId: string, baseBlobId: string, baseData: Buffer, targetData: Buffer, chainDepth: number | bigint): NativeJsonValue;
  call(method: "planPackSubstrateApplyGitBinaryDelta", baseData: Buffer, deltaData: Buffer): Buffer;
  call(method: "planPackSubstrateApplyPackDelta", baseData: Buffer, deltaData: Buffer, algorithm: string): Buffer;
  call(method: "planPackSubstrateWritePackArchive", packPath: string, packId: string, createdAt: string, members: NativeJsonValue): NativeJsonValue;
  call(method: "planPackSubstrateWriteTreePackArchive", packPath: string, packId: string, createdAt: string, members: NativeJsonValue): NativeJsonValue;
  call(method: "planPackSubstrateReadPackIndex", packPath: string): NativeJsonValue;
  call(method: "planPackSubstrateReadTreePackIndex", packPath: string): NativeJsonValue;
  call(method: "planPackSubstratePackHasEntry", packPath: string, entryName: string): boolean;
  call(method: "planPackSubstrateReadTreePackTree", packPath: string, treeId: string): NativeJsonValue;
  call(method: "planPackSubstrateSummarizePackArchives", root: string, packRows: NativeJsonValue): NativeJsonValue;
  call(method: "planPackSubstrateSummarizeTreePackArchives", root: string, packRows: NativeJsonValue): NativeJsonValue;
  call(method: "planPackSubstrateTreePackManifestPath", packPath: string, entryName: string): string;
  call(method: "planPackSubstrateReadTreePackTreeByOrdinal", packPath: string, entryOrdinal: number | bigint): NativeJsonValue;
  call(method: "planPackSubstrateBuildStorageValidationSummary", options: StorageValidationOptions): NativeJsonValue;
  call(method: "planPackSubstrateBuildTreeRecords", fileEntries: NativeJsonValue): NativeJsonValue;
  call(method: "planPackSubstrateBuildSnapshotId", options: BuildSnapshotIdOptions): NativeJsonValue;

  artifactBlobId(markdown: string): string;
  artifactCandidatesOpen(candidates: NativeJsonValue): NativeJsonValue;
  indexPlansByArtifactPath(plans: NativeJsonValue): NativeJsonValue;
  indexPlansByArtifactIdentity(plans: NativeJsonValue): NativeJsonValue;
  planArtifactIdentity(artifactPath: string, artifactSelector?: string | null): NativeJsonValue;
  planArtifactIdentityLabel(artifactPath: string, artifactSelector?: string | null): string;
  openGenericPlansMatchingBlobId(plans: NativeJsonValue, blobId: string): NativeJsonValue;
  openPlansMatchingSelector(plans: NativeJsonValue, selector: string): NativeJsonValue;
  localPlanFullyPublished(plan: NativeJsonValue): boolean;
  planHeadsEquivalent(left: NativeJsonValue, right: NativeJsonValue): boolean;
  planMatchesSyncArtifact(plan: NativeJsonValue, artifact: NativeJsonValue, requireTitleMatch?: boolean | null): boolean;
  planFilesystemNormalizeMarkdownArtifactPath(pathValue: string): string;
  planFilesystemIsMarkdownArtifactPath(pathValue: string): boolean;
  planFilesystemIsLineageOnlyMarkdownArtifactPath(pathValue: string): boolean;
  planFilesystemReadUtf8TextFile(pathValue: string): string;
  planFilesystemReadJsonFile(pathValue: string): NativeJsonValue;
  planFilesystemReadBinaryFile(pathValue: string): Buffer;
  planFilesystemPathIsProjectedOutForWorkspace(repoRoot: string, relPath: string, isWorktree?: boolean | null): boolean;
  planFilesystemWorkspacePathIsIgnored(repoRoot: string, pathValue: string, ignoreRulesText?: string | null): boolean;
  planFilesystemListVisibleWorkspacePaths(repoRoot: string, options?: VisibleWorkspaceOptions | null): string[];
  planFilesystemListVisibleMarkdownArtifactPaths(repoRoot: string, options?: VisibleWorkspaceOptions | null): string[];
  planFilesystemZipArchiveHasMember(pathValue: string, entryName: string): boolean;
  planFilesystemReadZipArchiveMember(pathValue: string, entryName: string): Buffer;
  planFilesystemResolveRepoArtifactPath(repoRoot: string, pathValue: string, allowMissing?: boolean | null): ResolvedRepoArtifactPath;
  repoStateReadRepoConfigJson(pathValue: string): NativeJsonValue;
  repoStateWriteRepoConfigJson(pathValue: string, payload: NativeJsonValue): void;
  repoStateReadWorktreeConfigJson(pathValue: string): NativeJsonValue;
  repoStateWriteWorktreeConfigJson(pathValue: string, payload: NativeJsonValue): void;
  repoStateReadWorktreeMetadataJson(pathValue: string): NativeJsonValue;
  repoStateWriteWorktreeMetadataJson(pathValue: string, payload: NativeJsonValue): void;
  call(method: "artifactBlobId", markdown: string): string;
  call(method: "artifactCandidatesOpen", candidates: NativeJsonValue): NativeJsonValue;
  call(method: "indexPlansByArtifactPath", plans: NativeJsonValue): NativeJsonValue;
  call(method: "indexPlansByArtifactIdentity", plans: NativeJsonValue): NativeJsonValue;
  call(method: "planArtifactIdentity", artifactPath: string, artifactSelector?: string | null): NativeJsonValue;
  call(method: "planArtifactIdentityLabel", artifactPath: string, artifactSelector?: string | null): string;
  call(method: "openGenericPlansMatchingBlobId", plans: NativeJsonValue, blobId: string): NativeJsonValue;
  call(method: "openPlansMatchingSelector", plans: NativeJsonValue, selector: string): NativeJsonValue;
  call(method: "localPlanFullyPublished", plan: NativeJsonValue): boolean;
  call(method: "planHeadsEquivalent", left: NativeJsonValue, right: NativeJsonValue): boolean;
  call(method: "planMatchesSyncArtifact", plan: NativeJsonValue, artifact: NativeJsonValue, requireTitleMatch?: boolean | null): boolean;
  call(method: "planFilesystemNormalizeMarkdownArtifactPath", pathValue: string): string;
  call(method: "planFilesystemIsMarkdownArtifactPath", pathValue: string): boolean;
  call(method: "planFilesystemIsLineageOnlyMarkdownArtifactPath", pathValue: string): boolean;
  call(method: "planFilesystemReadUtf8TextFile", pathValue: string): string;
  call(method: "planFilesystemReadJsonFile", pathValue: string): NativeJsonValue;
  call(method: "planFilesystemReadBinaryFile", pathValue: string): Buffer;
  call(method: "planFilesystemPathIsProjectedOutForWorkspace", repoRoot: string, relPath: string, isWorktree?: boolean | null): boolean;
  call(method: "planFilesystemWorkspacePathIsIgnored", repoRoot: string, pathValue: string, ignoreRulesText?: string | null): boolean;
  call(method: "planFilesystemListVisibleWorkspacePaths", repoRoot: string, options?: VisibleWorkspaceOptions | null): string[];
  call(method: "planFilesystemListVisibleMarkdownArtifactPaths", repoRoot: string, options?: VisibleWorkspaceOptions | null): string[];
  call(method: "planFilesystemZipArchiveHasMember", pathValue: string, entryName: string): boolean;
  call(method: "planFilesystemReadZipArchiveMember", pathValue: string, entryName: string): Buffer;
  call(method: "planFilesystemResolveRepoArtifactPath", repoRoot: string, pathValue: string, allowMissing?: boolean | null): ResolvedRepoArtifactPath;
  call(method: "repoStateReadRepoConfigJson", pathValue: string): NativeJsonValue;
  call(method: "repoStateWriteRepoConfigJson", pathValue: string, payload: NativeJsonValue): void;
  call(method: "repoStateReadWorktreeConfigJson", pathValue: string): NativeJsonValue;
  call(method: "repoStateWriteWorktreeConfigJson", pathValue: string, payload: NativeJsonValue): void;
  call(method: "repoStateReadWorktreeMetadataJson", pathValue: string): NativeJsonValue;
  call(method: "repoStateWriteWorktreeMetadataJson", pathValue: string, payload: NativeJsonValue): void;

  currentCoreSourceFingerprint(coreRepoRoot: string): string;
  currentServerSourceFingerprint(serverCoreRepoRoot: string): string;
  currentSourceNativeCacheContract(request: CurrentSourceCacheRequest): NativeJsonValue;
  currentSourceCacheMarkBuilding(request: CurrentSourceCacheRequest, sourceMtimeNs: number | bigint, extraJson?: string | null): NativeJsonValue;
  currentSourceCacheActivate(request: CurrentSourceCacheRequest, sourceMtimeNs: number | bigint, options?: CurrentSourceCacheActivateOptions | null): NativeJsonValue;
  currentSourceCacheReleaseLease(leasePath: string, namespaceRoot: string, removeUnleasedReady?: boolean | null): NativeJsonValue;
  currentSourceCachePrune(namespaceRoot: string, options?: CurrentSourceCachePruneOptions | null): NativeJsonValue;
  currentSourceCacheSeedCanonical(request: CurrentSourceCacheRequest, repoRoot: string, canonicalRepoRoot: string, coreSourceMtimeNs: number | bigint): NativeJsonValue;
  currentSourceCacheExtensionFresh(request: CurrentSourceExtensionFreshnessRequest): NativeJsonValue;
  currentSourceCacheBinaryFresh(request: CurrentSourceBinaryFreshnessRequest): NativeJsonValue;
  currentSourceCacheCoreFingerprint(repoRoot: string): NativeJsonValue;
  currentSourceCacheServerFingerprint(repoRoot: string): NativeJsonValue;
  currentSourceCacheCoreMtime(repoRoot: string): NativeJsonValue;
  currentSourceCacheServerMtime(repoRoot: string): NativeJsonValue;
  formatCurrentSourceCacheContract(payload: NativeJsonValue): string;
  formatCurrentSourceCacheMarkBuilding(payload: NativeJsonValue): string;
  formatCurrentSourceCacheActivate(payload: NativeJsonValue): string;
  formatCurrentSourceCacheReleaseLease(payload: NativeJsonValue): string;
  formatCurrentSourceCachePrune(payload: NativeJsonValue): string;
  formatCurrentSourceCacheSeedCanonical(payload: NativeJsonValue): string;
  formatCurrentSourceCacheExtensionFresh(payload: NativeJsonValue): string;
  formatCurrentSourceCacheBinaryFresh(payload: NativeJsonValue): string;
  formatCurrentSourceCacheCoreFingerprint(payload: NativeJsonValue): string;
  formatCurrentSourceCacheServerFingerprint(payload: NativeJsonValue): string;
  formatCurrentSourceCacheCoreMtime(payload: NativeJsonValue): string;
  formatCurrentSourceCacheServerMtime(payload: NativeJsonValue): string;
  extractPlanItems(markdown?: string | null): ParsedPlanItem[];
  listPlanSectionRefs(markdown?: string | null): ParsedPlanReference[];
  extractPlanSection(markdown?: string | null, planRef?: string | null): ExtractedPlanSection | null;
  findPlanItem(markdown?: string | null, planItemRef?: string | null): ParsedPlanItem | null;
  normalizePlanItems(items?: PlanItemInput[] | null): ParsedPlanItem[];
  findPlanItemInItems(items?: PlanItemInput[] | null, planItemRef?: string | null): ParsedPlanItem | null;
  normalizeTaskWorkflowHttpCompatibility(payloadJson: string): NativeJsonValue;
  normalizeTaskWorkflowHttpReadiness(payloadJson: string): NativeJsonValue;
  planItemsPayload(plan: NativeJsonValue, revision?: NativeJsonValue | null): NativeJsonValue;
  localPlanPublishShadow(plan?: NativeJsonValue | null): NativeJsonValue;
  planDispatchSummary(plan: NativeJsonValue, options?: PlanDispatchOptions | null): NativeJsonValue;
  computeTaskableItems(plan: NativeJsonValue, options?: PlanDispatchOptions | null): NativeJsonValue;
  validateDispatchLegality(plan: NativeJsonValue, planItemRef?: string | null, options?: PlanDispatchOptions | null): NativeJsonValue;
  planTaskLinkIndexes(tasks?: NativeJsonValue | null): PlanTaskLinkIndexes;
  planCandidatesPayload(summaries?: NativeJsonValue | null, options?: PlanCandidatesPayloadOptions | null): NativeJsonValue;
  call(method: "currentCoreSourceFingerprint", coreRepoRoot: string): string;
  call(method: "currentServerSourceFingerprint", serverCoreRepoRoot: string): string;
  call(method: "currentSourceNativeCacheContract", request: CurrentSourceCacheRequest): NativeJsonValue;
  call(method: "currentSourceCacheMarkBuilding", request: CurrentSourceCacheRequest, sourceMtimeNs: number | bigint, extraJson?: string | null): NativeJsonValue;
  call(method: "currentSourceCacheActivate", request: CurrentSourceCacheRequest, sourceMtimeNs: number | bigint, options?: CurrentSourceCacheActivateOptions | null): NativeJsonValue;
  call(method: "currentSourceCacheReleaseLease", leasePath: string, namespaceRoot: string, removeUnleasedReady?: boolean | null): NativeJsonValue;
  call(method: "currentSourceCachePrune", namespaceRoot: string, options?: CurrentSourceCachePruneOptions | null): NativeJsonValue;
  call(method: "currentSourceCacheSeedCanonical", request: CurrentSourceCacheRequest, repoRoot: string, canonicalRepoRoot: string, coreSourceMtimeNs: number | bigint): NativeJsonValue;
  call(method: "currentSourceCacheExtensionFresh", request: CurrentSourceExtensionFreshnessRequest): NativeJsonValue;
  call(method: "currentSourceCacheBinaryFresh", request: CurrentSourceBinaryFreshnessRequest): NativeJsonValue;
  call(method: "currentSourceCacheCoreFingerprint", repoRoot: string): NativeJsonValue;
  call(method: "currentSourceCacheServerFingerprint", repoRoot: string): NativeJsonValue;
  call(method: "currentSourceCacheCoreMtime", repoRoot: string): NativeJsonValue;
  call(method: "currentSourceCacheServerMtime", repoRoot: string): NativeJsonValue;
  call(method: "formatCurrentSourceCacheContract", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheMarkBuilding", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheActivate", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheReleaseLease", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCachePrune", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheSeedCanonical", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheExtensionFresh", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheBinaryFresh", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheCoreFingerprint", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheServerFingerprint", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheCoreMtime", payload: NativeJsonValue): string;
  call(method: "formatCurrentSourceCacheServerMtime", payload: NativeJsonValue): string;
  call(method: "extractPlanItems", markdown?: string | null): ParsedPlanItem[];
  call(method: "listPlanSectionRefs", markdown?: string | null): ParsedPlanReference[];
  call(method: "extractPlanSection", markdown?: string | null, planRef?: string | null): ExtractedPlanSection | null;
  call(method: "findPlanItem", markdown?: string | null, planItemRef?: string | null): ParsedPlanItem | null;
  call(method: "normalizePlanItems", items?: PlanItemInput[] | null): ParsedPlanItem[];
  call(method: "findPlanItemInItems", items?: PlanItemInput[] | null, planItemRef?: string | null): ParsedPlanItem | null;
  call(method: "normalizeTaskWorkflowHttpCompatibility", payloadJson: string): NativeJsonValue;
  call(method: "normalizeTaskWorkflowHttpReadiness", payloadJson: string): NativeJsonValue;
  call(method: "planItemsPayload", plan: NativeJsonValue, revision?: NativeJsonValue | null): NativeJsonValue;
  call(method: "localPlanPublishShadow", plan?: NativeJsonValue | null): NativeJsonValue;
  call(method: "planDispatchSummary", plan: NativeJsonValue, options?: PlanDispatchOptions | null): NativeJsonValue;
  call(method: "computeTaskableItems", plan: NativeJsonValue, options?: PlanDispatchOptions | null): NativeJsonValue;
  call(method: "validateDispatchLegality", plan: NativeJsonValue, planItemRef?: string | null, options?: PlanDispatchOptions | null): NativeJsonValue;
  call(method: "planTaskLinkIndexes", tasks?: NativeJsonValue | null): PlanTaskLinkIndexes;
  call(method: "planCandidatesPayload", summaries?: NativeJsonValue | null, options?: PlanCandidatesPayloadOptions | null): NativeJsonValue;

  constructor(options?: { addonPath?: string | URL });
  addonPath: string | null;
  resolveAddonPath(): string;
  loadAddon(): NativeAddon;
  resolveCallable(name: "HttpTaskRemote"): NativeHttpTaskRemoteConstructor;
  resolveCallable(name: "HttpWorkflowCloseoutRemote"): NativeHttpWorkflowCloseoutRemoteConstructor;
  resolveCallable(name: "PlanRemoteTransportConfig"): NativePlanRemoteTransportConfigConstructor;
  resolveCallable(name: "PlanHttpClientManager"): NativePlanHttpClientManagerConstructor;
  resolveCallable(name: "TaskWorkflowHttpClientManager"): NativeTaskWorkflowHttpClientManagerConstructor;
  resolveCallable(name: "WorkspaceCommandLock"): NativeWorkspaceCommandLockConstructor;
  resolveCallable(name: keyof NativeAddon | string): (...args: unknown[]) => unknown;
  call(method: "bindingInfoJson"): string;
  call(method: "agentWorkerCapabilitiesJson"): string;
  call(
    name: "agentManagementJson" | "agentWorkerTransactionJson",
    requestJson: string,
  ): string;
  call(method: "taskWorkflowInit", repoRoot: string, options?: InitOptions | null): NativeJsonValue;
  call(method: "taskWorkflowConfigShow", repoRoot: string): NativeJsonValue;
  call(method: "taskWorkflowConfigSet", repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  call(method: "taskWorkflowStatus", repoRoot: string): NativeJsonValue;
  call(method: "taskWorkflowRemoteAdd", repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  call(method: "taskWorkflowRemoteList", repoRoot: string): NativeJsonValue;
  call(method: "taskWorkflowDoctorRuntimeRoot", repoRoot: string): NativeJsonValue;
  call(method: "taskWorkflowDoctorPlanAuthority"): NativeJsonValue;
  call(method: "taskWorkflowTaskStart", repoRoot: string, title: string, intent: string, options?: ScopeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowTaskList", repoRoot: string, options?: TaskListOptions | null): NativeJsonValue[];
  call(method: "taskWorkflowTaskShow", repoRoot: string, taskId: string, options?: ScopeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowSnapshotCreate", repoRoot: string, message?: string | null): NativeJsonValue;
  call(method: "taskWorkflowSnapshotList", repoRoot: string): NativeJsonValue;
  call(method: "taskWorkflowSnapshotShow", repoRoot: string, snapshotId: string): NativeJsonValue;
  call(method: "planPortsNormalizePlanStoreReadRequest", payloadJson: string): NativeJsonValue;
  call(method: "validatePlanningSessionJoinPayload", payloadJson: string): NativeJsonValue;
  call(method: "planPortsNormalizePlanRemoteTransport", payloadJson: string): NativeJsonValue;
  call(method: "planPortsNormalizePlanRemoteRequest", payloadJson: string): NativeJsonValue;
  call(method: "planPortsNormalizeArtifactResolverRequest", payloadJson: string): NativeJsonValue;
  call(method: "planPortsNormalizeArtifactPublishRequest", payloadJson: string): NativeJsonValue;
  call(method: "planPortsNormalizeLinkedTaskLookupPayload", payloadJson: string): NativeJsonValue;
  call(method: "planPortsNormalizePlanConfigRuntimeFacts", payloadJson: string): NativeJsonValue;
  call(method: "planPortsNormalizePlanConnectionManagerStats", payloadJson: string): NativeJsonValue;
  call(method: "planConfigRuntimeNormalizeSelectionRequest", payloadJson: string): NativeJsonValue;
  call(method: "planConfigRuntimeBuildSelectionFacts", payloadJson: string): NativeJsonValue;
  call(method: "planConfigRuntimeNormalizeSelectionFacts", payloadJson: string): NativeJsonValue;
  call(method: "planConfigRuntimeNormalizeCompatibility", payloadJson: string): NativeJsonValue;
  call(method: "planConfigRuntimeNormalizeReadiness", payloadJson: string): NativeJsonValue;
  call(method: "planConfigRuntimeNormalizeDoctor", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsNormalizeRequest", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsNormalizeBackendIdentity", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsNormalizeCompatibility", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsNormalizeReadiness", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsNormalizeDoctor", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsBuildBackendIdentityFacts", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsBuildStorageReadinessFacts", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsBuildCompatibilityStatus", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsBuildReadinessStatus", payloadJson: string): NativeJsonValue;
  call(method: "planDiagnosticsBuildDoctorFacts", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationNormalizeListRequest", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationBuildList", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationNormalizeShowRequest", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationBuildShow", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationNormalizeRevisionsRequest", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationBuildRevisions", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationNormalizeItemsRequest", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationBuildItems", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationNormalizeCandidatesRequest", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationBuildCandidates", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationNormalizeInspectRequest", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationBuildInspect", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationNormalizeSyncRequest", payloadJson: string): NativeJsonValue;
  call(method: "planApplicationBuildSync", payloadJson: string): NativeJsonValue;
  call(method: "planCommandNormalizeListRequest", payloadJson: string): NativeJsonValue;
  call(method: "planCommandBuildList", payloadJson: string): NativeJsonValue;
  call(method: "planCommandNormalizeShowRequest", payloadJson: string): NativeJsonValue;
  call(method: "planCommandBuildShow", payloadJson: string): NativeJsonValue;
  call(method: "planCommandNormalizeRevisionsRequest", payloadJson: string): NativeJsonValue;
  call(method: "planCommandBuildRevisions", payloadJson: string): NativeJsonValue;
  call(method: "planCommandNormalizeItemsRequest", payloadJson: string): NativeJsonValue;
  call(method: "planCommandBuildItems", payloadJson: string): NativeJsonValue;
  call(method: "planCommandNormalizeCandidatesRequest", payloadJson: string): NativeJsonValue;
  call(method: "planCommandBuildCandidates", payloadJson: string): NativeJsonValue;
  call(method: "planCommandNormalizeInspectRequest", payloadJson: string): NativeJsonValue;
  call(method: "planCommandBuildInspect", payloadJson: string): NativeJsonValue;
  call(method: "planCommandNormalizeSyncRequest", payloadJson: string): NativeJsonValue;
  call(method: "planCommandBuildSync", payloadJson: string): NativeJsonValue;
  call(method: "planCommandExecuteList", payloadJson: string): NativeJsonValue;
  call(method: "planCommandExecuteShow", payloadJson: string): NativeJsonValue;
  call(method: "planCommandExecuteRevisions", payloadJson: string): NativeJsonValue;
  call(method: "planCommandExecuteItems", payloadJson: string): NativeJsonValue;
  call(method: "planCommandExecuteCandidates", payloadJson: string): NativeJsonValue;
  call(method: "planCommandExecuteInspect", payloadJson: string): NativeJsonValue;
  call(method: "planCommandExecuteSync", payloadJson: string): NativeJsonValue;
  call(method: "planProvenanceNormalizeRevisionProvenance", payloadJson: string): NativeJsonValue;
  call(method: "planProvenanceBuildRevisionProvenance", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityNormalizeTimestampRequest", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityNormalizeTimestamp", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityBuildTimestamp", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityNormalizeSequenceIdentityRequest", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityNormalizeSequenceIdentity", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityBuildSequenceIdentity", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityNormalizeWorkflowIdRequest", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityNormalizeWorkflowId", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityBuildWorkflowId", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityNormalizeTemporalOrdering", payloadJson: string): NativeJsonValue;
  call(method: "planTimeIdentityBuildTemporalOrdering", payloadJson: string): NativeJsonValue;
  call(method: "initializeRepository", repoRoot: string, options?: InitOptions | null): NativeJsonValue;
  call(method: "configureRepository", repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  call(method: "configurationError", payload: NativeJsonValue): string | null;
  call(method: "taskStart", repoRoot: string, request: TaskStartRequest): NativeJsonValue;
  call(method: "taskQuick", repoRoot: string, request: TaskQuickRequest): NativeJsonValue;
  call(method: "taskFinish", repoRoot: string, taskId: string, options?: TaskFinishOptions | null): NativeJsonValue;
  call(method: "taskFinishExitCode", payload: NativeJsonValue): number;
  call(method: "snapshotCapture", repoRoot: string, reference: string, options?: SnapshotCaptureOptions | null): NativeJsonValue;
  call(method: "projectTaskStart", payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  call(method: "formatTaskStart", payload: NativeJsonValue): string;
  call(method: "projectTaskFinish", payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  call(method: "formatTaskFinish", payload: NativeJsonValue, requested: string): string;
  call(method: "projectSnapshotCapture", payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  call(method: "formatSnapshotCapture", payload: NativeJsonValue): string;
  call(method: "projectStatus", payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  call(method: "encodeJson", value: NativeJsonValue, pretty?: boolean | null): string;
  call(method: "formatStatus", payload: NativeJsonValue): string;
  call(method: "formatDomainError", message: string): string;
  call(method: "domainErrorExitCode"): number;
  call(method: "decodeJson", text: string): NativeJsonValue;
  call(method: "planList", repoRoot: string, options?: ScopeOptions | null): NativeJsonValue[];
  call(method: "planShow", repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  call(method: "planRevisions", repoRoot: string, planId: string, options?: ScopeOptions | null): NativeJsonValue[];
  call(method: "planItems", repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  call(method: "planInspect", repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  call(method: "planCandidates", repoRoot: string, options?: PlanCandidatesOptions | null): NativeJsonValue;
  call(method: "planSync", repoRoot: string, target: string, options?: PlanSyncOptions | null): NativeJsonValue;
  call(method: "planSyncError", payload: NativeJsonValue): string | null;
  call(method: "formatPlanList", payload: NativeJsonValue, options?: PlanListOutputOptions | null): string;
  call(method: "formatPlanRevisions", payload: NativeJsonValue, planId: string, options?: PlanListOutputOptions | null): string;
  call(method: "formatPlanShow", payload: NativeJsonValue): string;
  call(method: "formatPlanItems", payload: NativeJsonValue): string;
  call(method: "formatPlanInspect", payload: NativeJsonValue): string;
  call(method: "formatPlanCandidates", payload: NativeJsonValue): string;
  call(method: "formatPlanSync", payload: NativeJsonValue): string;
  call(method: "workflowReady", repoRoot: string, taskId: string, options?: WorkflowReadyOptions | null): WorkflowPhaseResult;
  call(method: "workflowFinish", repoRoot: string, taskId: string, options?: WorkflowFinishOptions | null): WorkflowPhaseResult;
  call(method: "workflowReconcile", repoRoot: string, options?: WorkflowReconcileOptions | null): NativeJsonValue;
  call(method: "workflowGuide", topic?: string | null): NativeJsonValue;
  call(method: "projectWorkflowReady", repoRoot: string, payload: NativeJsonValue, taskId: string, options?: WorkflowOutputOptions | null): NativeJsonValue;
  call(method: "projectWorkflowFinish", repoRoot: string, payload: NativeJsonValue, taskId: string, options?: WorkflowOutputOptions | null): NativeJsonValue;
  call(method: "formatWorkflowReady", payload: NativeJsonValue, taskId: string): string;
  call(method: "formatWorkflowFinish", payload: NativeJsonValue, taskId: string): string;
  call(method: "formatWorkflowReconcile", payload: NativeJsonValue): string;
  call(method: "formatWorkflowGuide", payload: NativeJsonValue): string;
  call(method: "repositoryAuthorityRoot", repoRoot: string): string;
  call(method: "formatTaskQuick", payload: NativeJsonValue): string;
  call(method: "taskAudit", repoRoot: string, taskId: string, options?: ScopeOptions | null): NativeJsonValue;
  call(method: "formatTaskAudit", payload: NativeJsonValue, taskId: string): string;
  call(method: "taskAbandon", repoRoot: string, taskId: string, options?: ScopeOptions | null): WorkflowPhaseResult;
  call(method: "formatTaskAbandon", payload: NativeJsonValue, taskId: string): string;
  call(method: "formatInitializeRepository", payload: NativeJsonValue): string;
  call(method: "formatConfigurationShow", payload: NativeJsonValue): string;
  call(method: "formatConfiguration", payload: NativeJsonValue, requestPayload: NativeJsonValue): string;
  call(method: "unsetConfiguration", repoRoot: string, key: string): NativeJsonValue;
  call(method: "formatConfigurationUnset", payload: NativeJsonValue, key: string): string;
  call(method: "formatRemoteAdd", payload: NativeJsonValue): string;
  call(method: "formatRemoteList", payload: NativeJsonValue): string;
  call(method: "queueSummary", repoRoot: string, remoteName?: string | null): NativeJsonValue;
  call(method: "formatQueueSummary", payload: NativeJsonValue): string;
  call(method: "doctorMemoryRoot", repoRoot: string): NativeJsonValue;
  call(method: "doctorPlanAuthority", repoRoot: string): NativeJsonValue;
  call(method: "formatDoctorMemoryRoot", payload: NativeJsonValue): string;
  call(method: "formatDoctorRuntimeRoot", payload: NativeJsonValue): string;
  call(method: "formatDoctorPlanAuthority", payload: NativeJsonValue): string;
  call(method: "snapshotAncestry", repoRoot: string, selector: string, options?: SnapshotAncestryOptions | null): NativeJsonValue;
  call(method: "snapshotIsAncestor", repoRoot: string, older: string, newer: string): WorkflowPhaseResult;
  call(method: "snapshotMergeBase", repoRoot: string, left: string, right: string, all?: boolean | null): WorkflowPhaseResult;
  call(method: "formatSnapshotAncestry", payload: NativeJsonValue, all?: boolean | null): string;
  call(method: "formatSnapshotIsAncestor", payload: NativeJsonValue): string;
  call(method: "formatSnapshotMergeBase", payload: NativeJsonValue): string;
  call(method: "formatSnapshotQueryError", message: string): string;
  call(method: "snapshotQueryErrorExitCode"): number;
  call(method: "projectTaskList", payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  call(method: "formatTaskList", payload: NativeJsonValue, options?: TaskListOutputOptions | null): string;
  call(method: "formatTaskShow", payload: NativeJsonValue): string;
  call(method: "projectSnapshotList", payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  call(method: "formatSnapshotList", payload: NativeJsonValue, all?: boolean | null): string;
  call(method: "inspectSnapshot", repoRoot: string, selector: string, includeDelta?: boolean | null): SnapshotInspection;
  call(method: "formatSnapshotShow", inspection: SnapshotInspection, files?: boolean | null): string;
  call(method: "taskWorkflowLineList", repoRoot: string, options?: LineListOptions | null): NativeJsonValue;
  call(method: "lineInventory", repoRoot: string, options?: LineListOptions | null): LineInventory;
  call(method: "taskWorkflowLineCreate", repoRoot: string, name: string, options?: LineCreateOptions | null): NativeJsonValue;
  call(method: "taskWorkflowLineSwitch", repoRoot: string, name: string, options?: LineSwitchOptions | null): NativeJsonValue;
  call(method: "taskWorkflowLineShow", repoRoot: string, name?: string | null): NativeJsonValue;
  call(method: "lineShow", repoRoot: string, name?: string | null, remoteName?: string | null): NativeJsonValue;
  call(method: "taskWorkflowLineArchive", repoRoot: string, name: string, remoteName?: string | null): NativeJsonValue;
  call(method: "taskWorkflowLineRename", repoRoot: string, oldName: string, newName: string, remoteName?: string | null): NativeJsonValue;
  call(method: "taskWorkflowLineDelete", repoRoot: string, name: string, options?: LineDeleteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowLineSetHead", repoRoot: string, name: string, snapshotId?: string | null): NativeJsonValue;
  call(method: "lineMerge", repoRoot: string, options?: LineMergeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowLineCleanup", repoRoot: string, options?: LineCleanupOptions | null): NativeJsonValue;
  call(method: "formatLineList", inventory: LineInventory, options?: LineListOptions | null): string;
  call(method: "formatLineCreate", payload: NativeJsonValue): string;
  call(method: "formatLineSwitch", payload: NativeJsonValue): string;
  call(method: "formatLineShow", payload: NativeJsonValue): string;
  call(method: "formatLineArchive", payload: NativeJsonValue): string;
  call(method: "formatLineRename", payload: NativeJsonValue): string;
  call(method: "formatLineDelete", payload: NativeJsonValue): string;
  call(method: "formatLineMerge", payload: NativeJsonValue): string;
  call(method: "formatLineCleanup", payload: NativeJsonValue, idleFor: string, options?: LineCleanupOutputOptions | null): string;
  call(method: "repositoryShow", repoRoot: string, remoteName?: string | null): NativeJsonValue;
  call(method: "repositoryJobs", repoRoot: string, options?: RepositoryJobsOptions | null): NativeJsonValue;
  call(method: "repositoryCiCapabilities", repoRoot: string, remoteName?: string | null): NativeJsonValue;
  call(method: "repositoryRetire", repoRoot: string, remoteName?: string | null, abort?: boolean | null): NativeJsonValue;
  call(method: "repositoryRestore", repoRoot: string, remoteName?: string | null): NativeJsonValue;
  call(method: "formatRepositoryShow", payload: NativeJsonValue, remoteName?: string | null): string;
  call(method: "formatRepositoryJobs", payload: NativeJsonValue, options?: RepositoryJobsOptions | null): string;
  call(method: "formatRepositoryCiCapabilities", payload: NativeJsonValue, remoteName?: string | null): string;
  call(method: "formatRepositoryRetire", payload: NativeJsonValue): string;
  call(method: "formatRepositoryRestore", payload: NativeJsonValue): string;
  call(method: "taskWorkflowChangeList", root: string, options?: ScopeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowChangeShow", root: string, changeId: string, options?: ChangeQueryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowChangeRevert", root: string, changeId: string, options?: ChangeMutationOptions | null): NativeJsonValue;
  call(method: "taskWorkflowChangeReplay", root: string, changeId: string, onto: string, options?: ChangeMutationOptions | null): NativeJsonValue;
  call(method: "taskWorkflowChangeClose", root: string, changeId: string, options?: ScopeOptions | null): NativeJsonValue;
  call(method: "taskWorkflowChangePublish", root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPatchsetPublish", root: string, changeId: string, summary: string, options?: PatchsetPublishOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPatchsetList", root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPatchsetShow", root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPatchsetSelect", root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPatchsetPublishExplicit", root: string, changeId: string, baseSnapshotId: string, revisionSnapshotId: string, summary: string, options?: PatchsetPublishExplicitOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPatchsetCiStatus", root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPatchsetRerunCi", root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReviewTeamApprove", root: string, changeId: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReviewShow", root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReviewRequest", root: string, changeId: string, reviewerGroups: string[], options?: ReviewRequestOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReviewTaskApprove", root: string, changeId: string, patchsetId: string, message: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReviewRecord", root: string, changeId: string, action: string, options?: ReviewRecordOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReviewCodeSubmit", root: string, changeId: string, patchsetId: string, message: string, options?: ReviewCodeSubmitOptions | null): NativeJsonValue;
  call(method: "taskWorkflowReviewCodeTemplate", options?: ReviewCodeTemplateOptions | null): NativeJsonValue;
  call(method: "taskWorkflowAttestPut", root: string, options?: AttestationPutOptions | null): NativeJsonValue;
  call(method: "taskWorkflowAttestShow", root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPolicyEval", root: string, patchsetId: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPolicyShow", root: string, patchsetId: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPolicyWaive", root: string, patchsetId: string, ruleName: string, reason: string, options?: PolicyWaiveOptions | null): NativeJsonValue;
  call(method: "changeCreate", repoRoot: string, taskId: string, title: string, options?: ChangeCreateOptions | null): NativeJsonValue;
  call(method: "changeShow", repoRoot: string, reference: string, options?: ScopeOptions | null): NativeJsonValue;
  call(method: "changeRevert", repoRoot: string, reference: string, options?: ChangeOperationOptions | null): NativeJsonValue;
  call(method: "changeReplay", repoRoot: string, reference: string, onto?: string | null, options?: ChangeOperationOptions | null): NativeJsonValue;
  call(method: "changeClose", repoRoot: string, reference: string, options?: ScopeOptions | null): NativeJsonValue;
  call(method: "changePublish", repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  call(method: "patchsetPublish", repoRoot: string, reference: string, summary: string, options?: PatchsetPublishOptions | null): NativeJsonValue;
  call(method: "patchsetList", repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  call(method: "formatChangeCreate", payload: NativeJsonValue): string;
  call(method: "formatChangeShow", payload: NativeJsonValue): string;
  call(method: "formatChangeRevert", payload: NativeJsonValue): string;
  call(method: "formatChangeReplay", payload: NativeJsonValue): string;
  call(method: "formatChangeClose", payload: NativeJsonValue): string;
  call(method: "formatChangePublish", payload: NativeJsonValue): string;
  call(method: "formatPatchsetPublish", payload: NativeJsonValue): string;
  call(method: "formatPatchsetList", payload: NativeJsonValue): string;
  call(method: "formatPatchsetShow", payload: NativeJsonValue): string;
  call(method: "formatPatchsetSelect", payload: NativeJsonValue): string;
  call(method: "formatPatchsetCiStatus", payload: NativeJsonValue): string;
  call(method: "formatPatchsetRerunCi", payload: NativeJsonValue): string;
  call(method: "projectChangeList", payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  call(method: "formatChangeList", payload: NativeJsonValue, options?: ChangeListOutputOptions | null): string;
  call(method: "workspaceCommandLockPath", repoRoot: string): string;
  call(method: "taskWorkflowSnapshotChain", repoRoot: string, snapshotId: string): NativeJsonValue;
  call(method: "taskWorkflowSnapshotExists", repoRoot: string, snapshotId: string): boolean;
  call(method: "taskWorkflowBlobReadBytes", repoRoot: string, blobId: string): Buffer;
  call(method: "taskWorkflowBlobEnsureBytes", repoRoot: string, data: Buffer, pathHint?: string | null): string;
  call(method: "taskWorkflowWorkspaceDelta", repoRoot: string, snapshotId?: string | null): NativeJsonValue;
  call(method: "taskWorkflowWorkspaceRestore", repoRoot: string, options?: WorkspaceRestoreOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorkspaceRestorePaths", repoRoot: string, paths: string[], options?: WorkspaceRestoreOptions | null): NativeJsonValue;
  call(method: "taskWorkflowSnapshotDiff", repoRoot: string, older: string, newer: string, options?: SnapshotDiffOptions | null): NativeJsonValue;
  call(method: "taskWorkflowSnapshotRevert", repoRoot: string, snapshotId: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  call(method: "taskWorkflowSnapshotReplay", repoRoot: string, snapshotId: string, onto: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  call(method: "taskWorkflowSnapshotCreateExplicit", repoRoot: string, repoName: string, lineName: string, options?: SnapshotExplicitCaptureOptions | null): NativeJsonValue;
  call(method: "taskWorkflowSnapshotRestoreLines", repoRoot: string, snapshotId: string, path: string, options?: SnapshotRestoreLinesOptions | null): NativeJsonValue;
  call(method: "snapshotDiff", repoRoot: string, olderSelector: string, newerSelector: string, options?: SnapshotDiffOptions | null): NativeJsonValue;
  call(method: "snapshotRevert", repoRoot: string, selector: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  call(method: "snapshotReplay", repoRoot: string, selector: string, onto?: string | null, options?: SnapshotMutationOptions | null): NativeJsonValue;
  call(method: "formatSnapshotDiff", payload: NativeJsonValue): string;
  call(method: "formatSnapshotRevert", payload: NativeJsonValue): string;
  call(method: "formatSnapshotReplay", payload: NativeJsonValue): string;
  call(method: "formatSnapshotRestoreLines", payload: NativeJsonValue): string;
  call(method: "taskWorkflowWorktreeGet", root: string, options?: WorktreeGetOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeStatus", root: string, options?: WorktreeStatusOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeRestore", root: string, options?: WorktreeRestoreOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeList", root: string, options?: WorktreeRefreshOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeTouchUsage", root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeDoctor", root: string, options?: WorktreeRefreshOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeCleanupCandidates", root: string, options?: WorktreeCleanupCandidatesOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeCleanup", root: string, options?: WorktreeCleanupOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreePruneStale", root: string, options?: WorktreeDryRunOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeRemove", root: string, names: string[], options?: WorktreeRemoveOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeBindExisting", root: string, worktreeName: string, options?: WorktreeBindExistingOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeSync", root: string, options?: WorktreeSyncOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeSyncAll", root: string, options?: WorktreeMutationOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeRecreate", root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeRestoreOwnedHead", root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreePreviewRebase", root: string, options?: WorktreeRebaseOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeRebase", root: string, options?: WorktreeRebaseOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeContinueRebase", root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  call(method: "taskWorkflowWorktreeAbortRebase", root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  call(method: "taskWorkflowAuthWhoami", repoRoot: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowAuthBindings", repoRoot: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  call(method: "taskWorkflowAuthGrant", repoRoot: string, actorIdentity: string, roles: string[], options?: RemoteRepositoryOptions | null): NativeJsonValue;
  call(method: "formatAuthWhoami", payload: NativeJsonValue): string;
  call(method: "formatAuthBindings", payload: NativeJsonValue): string;
  call(method: "formatAuthGrant", payload: NativeJsonValue): string;
  call(method: "stashSave", repoRoot: string, options?: StashSaveOptions | null): NativeJsonValue;
  call(method: "taskWorkflowStashList", repoRoot: string): NativeJsonValue;
  call(method: "taskWorkflowStashShow", repoRoot: string, stashId: string): NativeJsonValue;
  call(method: "stashApply", repoRoot: string, stashId: string, force?: boolean | null): NativeJsonValue;
  call(method: "stashPop", repoRoot: string, stashId: string, force?: boolean | null): NativeJsonValue;
  call(method: "stashDrop", repoRoot: string, stashId: string): NativeJsonValue;
  call(method: "taskWorkflowStashDrop", repoRoot: string, stashId: string): NativeJsonValue;
  call(method: "formatStashSave", payload: NativeJsonValue): string;
  call(method: "formatStashList", payload: NativeJsonValue): string;
  call(method: "formatStashShow", payload: NativeJsonValue): string;
  call(method: "formatStashApply", payload: NativeJsonValue): string;
  call(method: "formatStashPop", payload: NativeJsonValue): string;
  call(method: "formatStashDrop", payload: NativeJsonValue): string;
  call(method: "tagCreate", repoRoot: string, name: string, message: string, snapshotId?: string | null): NativeJsonValue;
  call(method: "tagList", repoRoot: string): NativeJsonValue[];
  call(method: "tagShow", repoRoot: string, name: string): NativeJsonValue;
  call(method: "tagDelete", repoRoot: string, name: string): NativeJsonValue;
  call(method: "resolveSnapshotRef", repoRoot: string, value: string): string;
  call(method: "formatTagCreate", payload: NativeJsonValue): string;
  call(method: "formatTagList", payload: NativeJsonValue): string;
  call(method: "formatTagShow", payload: NativeJsonValue): string;
  call(method: "formatTagDelete", payload: NativeJsonValue): string;
  call(method: "reviewShow", repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  call(method: "reviewTeamRequest", repoRoot: string, reference: string, reviewerGroups: string[], options?: ReviewRequestOptions | null): NativeJsonValue;
  call(method: "reviewTeamApprove", repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  call(method: "reviewTeamRequestChanges", repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  call(method: "reviewTeamComment", repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  call(method: "reviewTeamDefer", repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  call(method: "reviewTaskApprove", repoRoot: string, reference: string, patchsetSelector: string, message: string, remoteName?: string | null): ReviewTaskOperationResult;
  call(method: "reviewTaskRequestChanges", repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  call(method: "reviewTaskComment", repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  call(method: "reviewTaskDefer", repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  call(method: "reviewCodeSubmit", repoRoot: string, reference: string, patchsetId: string, message: string, options?: ReviewCodeSubmitOptions | null): NativeJsonValue;
  call(method: "formatReviewTaskApprove", operation: ReviewTaskOperationResult): string;
  call(method: "formatReviewTaskRequestChanges", operation: ReviewTaskOperationResult): string;
  call(method: "formatReviewTaskComment", operation: ReviewTaskOperationResult): string;
  call(method: "formatReviewTaskDefer", operation: ReviewTaskOperationResult): string;
  call(method: "formatReviewTeamRequest", payload: NativeJsonValue): string;
  call(method: "formatReviewTeamApprove", payload: NativeJsonValue): string;
  call(method: "formatReviewTeamRequestChanges", payload: NativeJsonValue): string;
  call(method: "formatReviewTeamComment", payload: NativeJsonValue): string;
  call(method: "formatReviewTeamDefer", payload: NativeJsonValue): string;
  call(method: "formatReviewShow", payload: NativeJsonValue): string;
  call(method: "formatReviewCodeSubmit", payload: NativeJsonValue): string;
  call(method: "formatReviewCodeTemplate", payload: NativeJsonValue): string;
  call(method: "projectReviewTaskOperation", operation: ReviewTaskOperationResult): NativeJsonValue;
  call(method: "projectReviewShow", payload: NativeJsonValue): NativeJsonValue;
  call(method: "validateReviewPatchsetSelector", value: string): string;
  call(method: "validatePublicPatchsetArgument", value: string): string;
  call(method: "formatAttestPut", payload: NativeJsonValue): string;
  call(method: "formatAttestShow", payload: NativeJsonValue): string;
  call(method: "formatPolicyEval", payload: NativeJsonValue): string;
  call(method: "formatPolicyShow", payload: NativeJsonValue): string;
  call(method: "formatPolicyWaive", payload: NativeJsonValue): string;
  call(method: "projectPolicyShow", payload: NativeJsonValue): NativeJsonValue;
  call(method: "workspaceDiff", repoRoot: string, paths?: string[] | null): NativeJsonValue;
  call(method: "renderWorkspaceDiffNames", payload: NativeJsonValue): string;
  call(method: "renderWorkspaceDiffStat", payload: NativeJsonValue): string;
  call(method: "renderWorkspaceDiffText", payload: NativeJsonValue): string;
  call(method: "taskWorkflowBlame", repoRoot: string, path: string, options?: BlameOptions | null): NativeJsonValue;
  call(method: "formatBlame", payload: NativeJsonValue): string;
  call(method: "pullRepository", root: string, options?: PullOptions | null): NativeJsonValue;
  call(method: "pushRepository", root: string, options?: RemoteLineOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPull", root: string, options?: PullOptions | null): NativeJsonValue;
  call(method: "taskWorkflowPush", root: string, options?: RemoteLineOptions | null): NativeJsonValue;
  call(method: "taskWorkflowUploadSnapshotChain", root: string, snapshotId: string, options?: SnapshotUploadOptions | null): NativeJsonValue;
  call(method: "formatPull", payload: NativeJsonValue): string;
  call(method: "formatPush", payload: NativeJsonValue): string;
  call(method: "projectRemoteSync", payload: NativeJsonValue): NativeJsonValue;
  call(method: "worktreeShow", root: string, name?: string | null): NativeJsonValue;
  call(method: "worktreePath", root: string, name?: string | null): NativeJsonValue;
  call(method: "worktreeRestore", root: string, options?: WorktreeRestoreOptions | null): NativeJsonValue;
  call(method: "worktreeCleanup", root: string, options?: WorktreeCleanupCommandOptions | null): NativeJsonValue;
  call(method: "worktreePruneStale", root: string, options?: WorktreePruneCommandOptions | null): NativeJsonValue;
  call(method: "worktreeSync", root: string, options?: WorktreeSyncCommandOptions | null): NativeJsonValue;
  call(method: "worktreeRecreate", root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  call(method: "worktreeRecoverTask", root: string, taskId: string, options?: WorktreeRecoverTaskOptions | null): NativeJsonValue;
  call(method: "worktreeRestoreOwnedHead", root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  call(method: "worktreeRebase", root: string, options?: WorktreeRebaseCommandOptions | null): NativeJsonValue;
  call(method: "worktreeRemove", root: string, names: string[], options?: WorktreeRemoveCommandOptions | null): NativeJsonValue;
  call(method: "formatWorktreeStatus", payload: NativeJsonValue, verbose?: boolean | null): string;
  call(method: "formatWorktreePath", payload: NativeJsonValue, shellOutput?: boolean | null): string;
  call(method: "formatWorktreeShow", payload: NativeJsonValue): string;
  call(method: "formatWorktreeList", payload: NativeJsonValue): string;
  call(method: "formatWorktreeDoctor", payload: NativeJsonValue): string;
  call(method: "formatWorktreeCleanupCandidates", payload: NativeJsonValue): string;
  call(method: "formatWorktreeCleanup", payload: NativeJsonValue): string;
  call(method: "formatWorktreePruneStale", payload: NativeJsonValue): string;
  call(method: "formatWorktreeSync", payload: NativeJsonValue): string;
  call(method: "formatWorktreeRestore", payload: NativeJsonValue): string;
  call(method: "formatWorktreeRecreate", payload: NativeJsonValue): string;
  call(method: "formatWorktreeRecoverTask", payload: NativeJsonValue): string;
  call(method: "formatWorktreeRestoreOwnedHead", payload: NativeJsonValue): string;
  call(method: "formatWorktreeRebase", payload: NativeJsonValue): string;
  call(method: "formatWorktreeRemove", payload: NativeJsonValue, allStale: boolean, dryRun: boolean, multiple: boolean): string;
  call(method: "projectWorktreePath", payload: NativeJsonValue): NativeJsonValue;
  call(method: "worktreeSyncExitCode", payload: NativeJsonValue, allWorktrees: boolean): number;
  call(method: "gitMigrate", root: string, stagingRoot: string, apply?: boolean | null): NativeJsonValue;
  call(method: "gitRecover", root: string, apply?: boolean | null): NativeJsonValue;
  call(method: "gitImport", root: string, source: string, options?: GitImportOptions | null): NativeJsonValue;
  call(method: "gitExport", root: string, target: string, options?: GitExportOptions | null): NativeJsonValue;
  call(method: "gitRestore", root: string, generation: string, gitObject: string, target: string): NativeJsonValue;
  call(method: "gitMirror", root: string, endpoint: string, direction: string, dryRun?: boolean | null): NativeJsonValue;
  call(method: "formatGitMigrate", payload: NativeJsonValue): string;
  call(method: "formatGitRecover", payload: NativeJsonValue): string;
  call(method: "formatGitImport", payload: NativeJsonValue): string;
  call(method: "formatGitExport", payload: NativeJsonValue): string;
  call(method: "formatGitRestore", payload: NativeJsonValue): string;
  call(method: "formatGitMirror", payload: NativeJsonValue): string;
  call(method: "gcStats", root: string): NativeJsonValue;
  call(method: "gcValidate", root: string): NativeJsonValue;
  call(method: "gcPrune", root: string, apply?: boolean | null): NativeJsonValue;
  call(method: "gcValidateExitCode", payload: NativeJsonValue): number;
  call(method: "formatGcStats", payload: NativeJsonValue): string;
  call(method: "formatGcValidate", payload: NativeJsonValue): string;
  call(method: "formatGcPrune", payload: NativeJsonValue): string;
  call(method: "policyProfileNames"): string[];
  call(method: "authorModeValues"): string[];
  call(method: "policyContentClassValues"): string[];
  call(method: "policyAuthorClassValues"): string[];
  call(method: "normalizeAuthorMode", value: string): string;
  call(method: "missingCodeReviewSummarySections", value?: string | null): string[];
  call(method: "isStructuredCodeReviewSummary", value?: string | null): boolean;
  call(method: "renderCodeReviewSummaryTemplate", style?: string | null): string;
  call(method: "codeReviewSummaryRequirementText", value?: string | null): string;
  call(method: "derivePolicyContentClass", changedPaths?: NativeJsonValue | null): string;
  call(method: "derivePolicyAuthorClass", authorMode?: string | null): string | null;
  call(method: "buildMinimumProvenance", authorMode: string, modelName?: string | null): [NativeJsonValue, NativeJsonValue];
  call(method: "policyProfile", name: string): NativeJsonValue;
  call(method: "normalizePolicy", policy?: NativeJsonValue | null, fallbackProfile?: string | null): NativeJsonValue;
  call(method: "resolveEffectivePolicy", policy?: NativeJsonValue | null, options?: EffectivePolicyOptions | null): NativeJsonValue;
  call(method: "policyToYaml", policy?: NativeJsonValue | null, fallbackProfile?: string | null): string;
  call(method: "parsePolicyYaml", text: string, fallbackProfile?: string | null): NativeJsonValue;
  call(method: "runtimeDataEnvValue"): string | null;
  call(method: "runtimeDataEnvName"): string | null;
  call(method: "resolveRuntimeDataRootWithSource", path?: string | null): [string, string];
  call(method: "resolveRuntimeDataRoot", path?: string | null): string;
  call(method: "resolveServerRuntimeRootWithSource", path?: string | null): [string, string];
  call(method: "resolveServerRuntimeRoot", path?: string | null): string;
  call(method: "encodeRefName", name: string): string;
  call(method: "decodeRefName", name: string): string;
  call(method: "normalizeTaskStatus", value?: string | null): string | null;
  call(method: "normalizeOptionalText", value?: string | null): string | null;
  call(method: "utcNow"): string;
  call(method: "generateNamespacedWorkflowId", family: string, namespacePrefix?: string | null): string;
  call(method: "normalizeIdNamespacePrefix", value?: string | null, defaultPrefix?: string | null): string;
  call(method: "workflowIdTokens", family: string, options?: WorkflowIdOptions | null): string[];
  call(method: "workflowOriginNamespacePrefix", originPrefix: string, namespacePrefix?: string | null): string;
  call(method: "workflowIdNamespacePrefixCandidates", options?: WorkflowNamespaceOptions | null): string[];
  call(method: "workflowIdNamespacePrefixForValue", family: string, value?: string | null, options?: WorkflowNamespaceOptions | null): string | null;
  call(method: "workflowIdMatchesAnyNamespacePrefix", family: string, value?: string | null, options?: WorkflowNamespaceOptions | null): boolean;
  call(method: "derivePatchsetId", changeId: string, patchsetNumber: number | bigint, namespacePrefix?: string | null): string;
  call(method: "generateWorkflowId", prefix: string): string;
  call(method: "taskStatusDetails", value?: string | null): TaskStatusDetails;
  call(method: "normalizePublicationState", value?: string | null): string | null;
  call(method: "publicationStateHasUnpublishedHead", value?: string | null): boolean;
  call(method: "workflowIdToken", family: string, namespacePrefix?: string | null): string;
  call(method: "workflowIdMatches", family: string, value?: string | null, options?: WorkflowIdOptions | null): boolean;
  call(method: "generateNamespacedSequenceId", family: string, number: number | bigint, options?: WorkflowSequenceOptions | null): string;
  call(method: "workflowSuccessEnvelope", kind: string, value?: string | null): WorkflowResultEnvelope;
  call(method: "workflowErrorEnvelope", kind: string, code: string, message: string, detail?: string | null): WorkflowResultEnvelope;
  call(method: "parsePlanMarkdown", markdown?: string | null): ParsedPlan;
  call(method: "extractPlanRefs", parsedPlan: ParsedPlanInput): PlanRefsResult;
  call(method: "computeSyncPruneDecisions", scope?: string | null, trackedArtifacts?: string[] | null, syncedArtifacts?: string[] | null): NativeJsonValue;
  call(method: "diffSnapshotManifests", oldFiles: NativeJsonValue, newFiles: NativeJsonValue, options?: ManifestIdentityOptions | null): NativeJsonValue;
  call(method: "snapshotDiffFromManifests", oldFiles: NativeJsonValue, newFiles: NativeJsonValue, options?: ManifestTextDiffOptions | null): NativeJsonValue;
  call(method: string, ...args: unknown[]): unknown;
  taskWorkflowLineList(repoRoot: string, options?: LineListOptions | null): NativeJsonValue;
  lineInventory(repoRoot: string, options?: LineListOptions | null): LineInventory;
  taskWorkflowLineCreate(repoRoot: string, name: string, options?: LineCreateOptions | null): NativeJsonValue;
  taskWorkflowLineSwitch(repoRoot: string, name: string, options?: LineSwitchOptions | null): NativeJsonValue;
  taskWorkflowLineShow(repoRoot: string, name?: string | null): NativeJsonValue;
  lineShow(repoRoot: string, name?: string | null, remoteName?: string | null): NativeJsonValue;
  taskWorkflowLineArchive(repoRoot: string, name: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowLineRename(repoRoot: string, oldName: string, newName: string, remoteName?: string | null): NativeJsonValue;
  taskWorkflowLineDelete(repoRoot: string, name: string, options?: LineDeleteOptions | null): NativeJsonValue;
  taskWorkflowLineSetHead(repoRoot: string, name: string, snapshotId?: string | null): NativeJsonValue;
  lineMerge(repoRoot: string, options?: LineMergeOptions | null): NativeJsonValue;
  taskWorkflowLineCleanup(repoRoot: string, options?: LineCleanupOptions | null): NativeJsonValue;
  formatLineList(inventory: LineInventory, options?: LineListOptions | null): string;
  formatLineCreate(payload: NativeJsonValue): string;
  formatLineSwitch(payload: NativeJsonValue): string;
  formatLineShow(payload: NativeJsonValue): string;
  formatLineArchive(payload: NativeJsonValue): string;
  formatLineRename(payload: NativeJsonValue): string;
  formatLineDelete(payload: NativeJsonValue): string;
  formatLineMerge(payload: NativeJsonValue): string;
  formatLineCleanup(payload: NativeJsonValue, idleFor: string, options?: LineCleanupOutputOptions | null): string;
  repositoryShow(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  repositoryJobs(repoRoot: string, options?: RepositoryJobsOptions | null): NativeJsonValue;
  repositoryCiCapabilities(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  repositoryRetire(repoRoot: string, remoteName?: string | null, abort?: boolean | null): NativeJsonValue;
  repositoryRestore(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  formatRepositoryShow(payload: NativeJsonValue, remoteName?: string | null): string;
  formatRepositoryJobs(payload: NativeJsonValue, options?: RepositoryJobsOptions | null): string;
  formatRepositoryCiCapabilities(payload: NativeJsonValue, remoteName?: string | null): string;
  formatRepositoryRetire(payload: NativeJsonValue): string;
  formatRepositoryRestore(payload: NativeJsonValue): string;
  taskWorkflowChangeList(root: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowChangeShow(root: string, changeId: string, options?: ChangeQueryOptions | null): NativeJsonValue;
  taskWorkflowChangeRevert(root: string, changeId: string, options?: ChangeMutationOptions | null): NativeJsonValue;
  taskWorkflowChangeReplay(root: string, changeId: string, onto: string, options?: ChangeMutationOptions | null): NativeJsonValue;
  taskWorkflowChangeClose(root: string, changeId: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowChangePublish(root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetPublish(root: string, changeId: string, summary: string, options?: PatchsetPublishOptions | null): NativeJsonValue;
  taskWorkflowPatchsetList(root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetShow(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetSelect(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetPublishExplicit(root: string, changeId: string, baseSnapshotId: string, revisionSnapshotId: string, summary: string, options?: PatchsetPublishExplicitOptions | null): NativeJsonValue;
  taskWorkflowPatchsetCiStatus(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPatchsetRerunCi(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowReviewTeamApprove(root: string, changeId: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  taskWorkflowReviewShow(root: string, changeId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowReviewRequest(root: string, changeId: string, reviewerGroups: string[], options?: ReviewRequestOptions | null): NativeJsonValue;
  taskWorkflowReviewTaskApprove(root: string, changeId: string, patchsetId: string, message: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowReviewRecord(root: string, changeId: string, action: string, options?: ReviewRecordOptions | null): NativeJsonValue;
  taskWorkflowReviewCodeSubmit(root: string, changeId: string, patchsetId: string, message: string, options?: ReviewCodeSubmitOptions | null): NativeJsonValue;
  taskWorkflowReviewCodeTemplate(options?: ReviewCodeTemplateOptions | null): NativeJsonValue;
  taskWorkflowAttestPut(root: string, options?: AttestationPutOptions | null): NativeJsonValue;
  taskWorkflowAttestShow(root: string, patchsetId: string, options?: RemoteOptions | null): NativeJsonValue;
  taskWorkflowPolicyEval(root: string, patchsetId: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowPolicyShow(root: string, patchsetId: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowPolicyWaive(root: string, patchsetId: string, ruleName: string, reason: string, options?: PolicyWaiveOptions | null): NativeJsonValue;
  changeCreate(repoRoot: string, taskId: string, title: string, options?: ChangeCreateOptions | null): NativeJsonValue;
  changeShow(repoRoot: string, reference: string, options?: ScopeOptions | null): NativeJsonValue;
  changeRevert(repoRoot: string, reference: string, options?: ChangeOperationOptions | null): NativeJsonValue;
  changeReplay(repoRoot: string, reference: string, onto?: string | null, options?: ChangeOperationOptions | null): NativeJsonValue;
  changeClose(repoRoot: string, reference: string, options?: ScopeOptions | null): NativeJsonValue;
  changePublish(repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  patchsetPublish(repoRoot: string, reference: string, summary: string, options?: PatchsetPublishOptions | null): NativeJsonValue;
  patchsetList(repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  formatChangeCreate(payload: NativeJsonValue): string;
  formatChangeShow(payload: NativeJsonValue): string;
  formatChangeRevert(payload: NativeJsonValue): string;
  formatChangeReplay(payload: NativeJsonValue): string;
  formatChangeClose(payload: NativeJsonValue): string;
  formatChangePublish(payload: NativeJsonValue): string;
  formatPatchsetPublish(payload: NativeJsonValue): string;
  formatPatchsetList(payload: NativeJsonValue): string;
  formatPatchsetShow(payload: NativeJsonValue): string;
  formatPatchsetSelect(payload: NativeJsonValue): string;
  formatPatchsetCiStatus(payload: NativeJsonValue): string;
  formatPatchsetRerunCi(payload: NativeJsonValue): string;
  projectChangeList(payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  formatChangeList(payload: NativeJsonValue, options?: ChangeListOutputOptions | null): string;
  workspaceCommandLockPath(repoRoot: string): string;
  taskWorkflowSnapshotChain(repoRoot: string, snapshotId: string): NativeJsonValue;
  taskWorkflowSnapshotExists(repoRoot: string, snapshotId: string): boolean;
  taskWorkflowBlobReadBytes(repoRoot: string, blobId: string): Buffer;
  taskWorkflowBlobEnsureBytes(repoRoot: string, data: Buffer, pathHint?: string | null): string;
  taskWorkflowWorkspaceDelta(repoRoot: string, snapshotId?: string | null): NativeJsonValue;
  taskWorkflowWorkspaceRestore(repoRoot: string, options?: WorkspaceRestoreOptions | null): NativeJsonValue;
  taskWorkflowWorkspaceRestorePaths(repoRoot: string, paths: string[], options?: WorkspaceRestoreOptions | null): NativeJsonValue;
  taskWorkflowSnapshotDiff(repoRoot: string, older: string, newer: string, options?: SnapshotDiffOptions | null): NativeJsonValue;
  taskWorkflowSnapshotRevert(repoRoot: string, snapshotId: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  taskWorkflowSnapshotReplay(repoRoot: string, snapshotId: string, onto: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  taskWorkflowSnapshotCreateExplicit(repoRoot: string, repoName: string, lineName: string, options?: SnapshotExplicitCaptureOptions | null): NativeJsonValue;
  taskWorkflowSnapshotRestoreLines(repoRoot: string, snapshotId: string, path: string, options?: SnapshotRestoreLinesOptions | null): NativeJsonValue;
  snapshotDiff(repoRoot: string, olderSelector: string, newerSelector: string, options?: SnapshotDiffOptions | null): NativeJsonValue;
  snapshotRevert(repoRoot: string, selector: string, options?: SnapshotMutationOptions | null): NativeJsonValue;
  snapshotReplay(repoRoot: string, selector: string, onto?: string | null, options?: SnapshotMutationOptions | null): NativeJsonValue;
  formatSnapshotDiff(payload: NativeJsonValue): string;
  formatSnapshotRevert(payload: NativeJsonValue): string;
  formatSnapshotReplay(payload: NativeJsonValue): string;
  formatSnapshotRestoreLines(payload: NativeJsonValue): string;
  taskWorkflowWorktreeGet(root: string, options?: WorktreeGetOptions | null): NativeJsonValue;
  taskWorkflowWorktreeStatus(root: string, options?: WorktreeStatusOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRestore(root: string, options?: WorktreeRestoreOptions | null): NativeJsonValue;
  taskWorkflowWorktreeList(root: string, options?: WorktreeRefreshOptions | null): NativeJsonValue;
  taskWorkflowWorktreeTouchUsage(root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  taskWorkflowWorktreeDoctor(root: string, options?: WorktreeRefreshOptions | null): NativeJsonValue;
  taskWorkflowWorktreeCleanupCandidates(root: string, options?: WorktreeCleanupCandidatesOptions | null): NativeJsonValue;
  taskWorkflowWorktreeCleanup(root: string, options?: WorktreeCleanupOptions | null): NativeJsonValue;
  taskWorkflowWorktreePruneStale(root: string, options?: WorktreeDryRunOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRemove(root: string, names: string[], options?: WorktreeRemoveOptions | null): NativeJsonValue;
  taskWorkflowWorktreeBindExisting(root: string, worktreeName: string, options?: WorktreeBindExistingOptions | null): NativeJsonValue;
  taskWorkflowWorktreeSync(root: string, options?: WorktreeSyncOptions | null): NativeJsonValue;
  taskWorkflowWorktreeSyncAll(root: string, options?: WorktreeMutationOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRecreate(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRestoreOwnedHead(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  taskWorkflowWorktreePreviewRebase(root: string, options?: WorktreeRebaseOptions | null): NativeJsonValue;
  taskWorkflowWorktreeRebase(root: string, options?: WorktreeRebaseOptions | null): NativeJsonValue;
  taskWorkflowWorktreeContinueRebase(root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  taskWorkflowWorktreeAbortRebase(root: string, options?: WorktreeNameOptions | null): NativeJsonValue;
  taskWorkflowAuthWhoami(repoRoot: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowAuthBindings(repoRoot: string, options?: RemoteRepositoryOptions | null): NativeJsonValue;
  taskWorkflowAuthGrant(repoRoot: string, actorIdentity: string, roles: string[], options?: RemoteRepositoryOptions | null): NativeJsonValue;
  formatAuthWhoami(payload: NativeJsonValue): string;
  formatAuthBindings(payload: NativeJsonValue): string;
  formatAuthGrant(payload: NativeJsonValue): string;
  stashSave(repoRoot: string, options?: StashSaveOptions | null): NativeJsonValue;
  taskWorkflowStashList(repoRoot: string): NativeJsonValue;
  taskWorkflowStashShow(repoRoot: string, stashId: string): NativeJsonValue;
  stashApply(repoRoot: string, stashId: string, force?: boolean | null): NativeJsonValue;
  stashPop(repoRoot: string, stashId: string, force?: boolean | null): NativeJsonValue;
  stashDrop(repoRoot: string, stashId: string): NativeJsonValue;
  taskWorkflowStashDrop(repoRoot: string, stashId: string): NativeJsonValue;
  formatStashSave(payload: NativeJsonValue): string;
  formatStashList(payload: NativeJsonValue): string;
  formatStashShow(payload: NativeJsonValue): string;
  formatStashApply(payload: NativeJsonValue): string;
  formatStashPop(payload: NativeJsonValue): string;
  formatStashDrop(payload: NativeJsonValue): string;
  tagCreate(repoRoot: string, name: string, message: string, snapshotId?: string | null): NativeJsonValue;
  tagList(repoRoot: string): NativeJsonValue[];
  tagShow(repoRoot: string, name: string): NativeJsonValue;
  tagDelete(repoRoot: string, name: string): NativeJsonValue;
  resolveSnapshotRef(repoRoot: string, value: string): string;
  formatTagCreate(payload: NativeJsonValue): string;
  formatTagList(payload: NativeJsonValue): string;
  formatTagShow(payload: NativeJsonValue): string;
  formatTagDelete(payload: NativeJsonValue): string;
  reviewShow(repoRoot: string, reference: string, remoteName?: string | null): NativeJsonValue;
  reviewTeamRequest(repoRoot: string, reference: string, reviewerGroups: string[], options?: ReviewRequestOptions | null): NativeJsonValue;
  reviewTeamApprove(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTeamRequestChanges(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTeamComment(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTeamDefer(repoRoot: string, reference: string, options?: ReviewTeamApproveOptions | null): NativeJsonValue;
  reviewTaskApprove(repoRoot: string, reference: string, patchsetSelector: string, message: string, remoteName?: string | null): ReviewTaskOperationResult;
  reviewTaskRequestChanges(repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  reviewTaskComment(repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  reviewTaskDefer(repoRoot: string, reference: string, options?: ReviewTaskOptions | null): ReviewTaskOperationResult;
  reviewCodeSubmit(repoRoot: string, reference: string, patchsetId: string, message: string, options?: ReviewCodeSubmitOptions | null): NativeJsonValue;
  formatReviewTaskApprove(operation: ReviewTaskOperationResult): string;
  formatReviewTaskRequestChanges(operation: ReviewTaskOperationResult): string;
  formatReviewTaskComment(operation: ReviewTaskOperationResult): string;
  formatReviewTaskDefer(operation: ReviewTaskOperationResult): string;
  formatReviewTeamRequest(payload: NativeJsonValue): string;
  formatReviewTeamApprove(payload: NativeJsonValue): string;
  formatReviewTeamRequestChanges(payload: NativeJsonValue): string;
  formatReviewTeamComment(payload: NativeJsonValue): string;
  formatReviewTeamDefer(payload: NativeJsonValue): string;
  formatReviewShow(payload: NativeJsonValue): string;
  formatReviewCodeSubmit(payload: NativeJsonValue): string;
  formatReviewCodeTemplate(payload: NativeJsonValue): string;
  projectReviewTaskOperation(operation: ReviewTaskOperationResult): NativeJsonValue;
  projectReviewShow(payload: NativeJsonValue): NativeJsonValue;
  validateReviewPatchsetSelector(value: string): string;
  validatePublicPatchsetArgument(value: string): string;
  formatAttestPut(payload: NativeJsonValue): string;
  formatAttestShow(payload: NativeJsonValue): string;
  formatPolicyEval(payload: NativeJsonValue): string;
  formatPolicyShow(payload: NativeJsonValue): string;
  formatPolicyWaive(payload: NativeJsonValue): string;
  projectPolicyShow(payload: NativeJsonValue): NativeJsonValue;
  workspaceDiff(repoRoot: string, paths?: string[] | null): NativeJsonValue;
  renderWorkspaceDiffNames(payload: NativeJsonValue): string;
  renderWorkspaceDiffStat(payload: NativeJsonValue): string;
  renderWorkspaceDiffText(payload: NativeJsonValue): string;
  taskWorkflowBlame(repoRoot: string, path: string, options?: BlameOptions | null): NativeJsonValue;
  formatBlame(payload: NativeJsonValue): string;
  pullRepository(root: string, options?: PullOptions | null): NativeJsonValue;
  pushRepository(root: string, options?: RemoteLineOptions | null): NativeJsonValue;
  taskWorkflowPull(root: string, options?: PullOptions | null): NativeJsonValue;
  taskWorkflowPush(root: string, options?: RemoteLineOptions | null): NativeJsonValue;
  taskWorkflowUploadSnapshotChain(root: string, snapshotId: string, options?: SnapshotUploadOptions | null): NativeJsonValue;
  formatPull(payload: NativeJsonValue): string;
  formatPush(payload: NativeJsonValue): string;
  projectRemoteSync(payload: NativeJsonValue): NativeJsonValue;
  worktreeShow(root: string, name?: string | null): NativeJsonValue;
  worktreePath(root: string, name?: string | null): NativeJsonValue;
  worktreeRestore(root: string, options?: WorktreeRestoreOptions | null): NativeJsonValue;
  worktreeCleanup(root: string, options?: WorktreeCleanupCommandOptions | null): NativeJsonValue;
  worktreePruneStale(root: string, options?: WorktreePruneCommandOptions | null): NativeJsonValue;
  worktreeSync(root: string, options?: WorktreeSyncCommandOptions | null): NativeJsonValue;
  worktreeRecreate(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  worktreeRecoverTask(root: string, taskId: string, options?: WorktreeRecoverTaskOptions | null): NativeJsonValue;
  worktreeRestoreOwnedHead(root: string, options?: WorktreeRecoveryOptions | null): NativeJsonValue;
  worktreeRebase(root: string, options?: WorktreeRebaseCommandOptions | null): NativeJsonValue;
  worktreeRemove(root: string, names: string[], options?: WorktreeRemoveCommandOptions | null): NativeJsonValue;
  formatWorktreeStatus(payload: NativeJsonValue, verbose?: boolean | null): string;
  formatWorktreePath(payload: NativeJsonValue, shellOutput?: boolean | null): string;
  formatWorktreeShow(payload: NativeJsonValue): string;
  formatWorktreeList(payload: NativeJsonValue): string;
  formatWorktreeDoctor(payload: NativeJsonValue): string;
  formatWorktreeCleanupCandidates(payload: NativeJsonValue): string;
  formatWorktreeCleanup(payload: NativeJsonValue): string;
  formatWorktreePruneStale(payload: NativeJsonValue): string;
  formatWorktreeSync(payload: NativeJsonValue): string;
  formatWorktreeRestore(payload: NativeJsonValue): string;
  formatWorktreeRecreate(payload: NativeJsonValue): string;
  formatWorktreeRecoverTask(payload: NativeJsonValue): string;
  formatWorktreeRestoreOwnedHead(payload: NativeJsonValue): string;
  formatWorktreeRebase(payload: NativeJsonValue): string;
  formatWorktreeRemove(payload: NativeJsonValue, allStale: boolean, dryRun: boolean, multiple: boolean): string;
  projectWorktreePath(payload: NativeJsonValue): NativeJsonValue;
  worktreeSyncExitCode(payload: NativeJsonValue, allWorktrees: boolean): number;
  gitMigrate(root: string, stagingRoot: string, apply?: boolean | null): NativeJsonValue;
  gitRecover(root: string, apply?: boolean | null): NativeJsonValue;
  gitImport(root: string, source: string, options?: GitImportOptions | null): NativeJsonValue;
  gitExport(root: string, target: string, options?: GitExportOptions | null): NativeJsonValue;
  gitRestore(root: string, generation: string, gitObject: string, target: string): NativeJsonValue;
  gitMirror(root: string, endpoint: string, direction: string, dryRun?: boolean | null): NativeJsonValue;
  formatGitMigrate(payload: NativeJsonValue): string;
  formatGitRecover(payload: NativeJsonValue): string;
  formatGitImport(payload: NativeJsonValue): string;
  formatGitExport(payload: NativeJsonValue): string;
  formatGitRestore(payload: NativeJsonValue): string;
  formatGitMirror(payload: NativeJsonValue): string;
  gcStats(root: string): NativeJsonValue;
  gcValidate(root: string): NativeJsonValue;
  gcPrune(root: string, apply?: boolean | null): NativeJsonValue;
  gcValidateExitCode(payload: NativeJsonValue): number;
  formatGcStats(payload: NativeJsonValue): string;
  formatGcValidate(payload: NativeJsonValue): string;
  formatGcPrune(payload: NativeJsonValue): string;
  policyProfileNames(): string[];
  authorModeValues(): string[];
  policyContentClassValues(): string[];
  policyAuthorClassValues(): string[];
  normalizeAuthorMode(value: string): string;
  missingCodeReviewSummarySections(value?: string | null): string[];
  isStructuredCodeReviewSummary(value?: string | null): boolean;
  renderCodeReviewSummaryTemplate(style?: string | null): string;
  codeReviewSummaryRequirementText(value?: string | null): string;
  derivePolicyContentClass(changedPaths?: NativeJsonValue | null): string;
  derivePolicyAuthorClass(authorMode?: string | null): string | null;
  buildMinimumProvenance(authorMode: string, modelName?: string | null): [NativeJsonValue, NativeJsonValue];
  policyProfile(name: string): NativeJsonValue;
  normalizePolicy(policy?: NativeJsonValue | null, fallbackProfile?: string | null): NativeJsonValue;
  resolveEffectivePolicy(policy?: NativeJsonValue | null, options?: EffectivePolicyOptions | null): NativeJsonValue;
  policyToYaml(policy?: NativeJsonValue | null, fallbackProfile?: string | null): string;
  parsePolicyYaml(text: string, fallbackProfile?: string | null): NativeJsonValue;
  runtimeDataEnvValue(): string | null;
  runtimeDataEnvName(): string | null;
  resolveRuntimeDataRootWithSource(path?: string | null): [string, string];
  resolveRuntimeDataRoot(path?: string | null): string;
  resolveServerRuntimeRootWithSource(path?: string | null): [string, string];
  resolveServerRuntimeRoot(path?: string | null): string;
  encodeRefName(name: string): string;
  decodeRefName(name: string): string;
  normalizeTaskStatus(value?: string | null): string | null;
  normalizeOptionalText(value?: string | null): string | null;
  utcNow(): string;
  generateNamespacedWorkflowId(family: string, namespacePrefix?: string | null): string;
  normalizeIdNamespacePrefix(value?: string | null, defaultPrefix?: string | null): string;
  workflowIdTokens(family: string, options?: WorkflowIdOptions | null): string[];
  workflowOriginNamespacePrefix(originPrefix: string, namespacePrefix?: string | null): string;
  workflowIdNamespacePrefixCandidates(options?: WorkflowNamespaceOptions | null): string[];
  workflowIdNamespacePrefixForValue(family: string, value?: string | null, options?: WorkflowNamespaceOptions | null): string | null;
  workflowIdMatchesAnyNamespacePrefix(family: string, value?: string | null, options?: WorkflowNamespaceOptions | null): boolean;
  derivePatchsetId(changeId: string, patchsetNumber: number | bigint, namespacePrefix?: string | null): string;
  generateWorkflowId(prefix: string): string;
  taskStatusDetails(value?: string | null): TaskStatusDetails;
  normalizePublicationState(value?: string | null): string | null;
  publicationStateHasUnpublishedHead(value?: string | null): boolean;
  workflowIdToken(family: string, namespacePrefix?: string | null): string;
  workflowIdMatches(family: string, value?: string | null, options?: WorkflowIdOptions | null): boolean;
  generateNamespacedSequenceId(family: string, number: number | bigint, options?: WorkflowSequenceOptions | null): string;
  workflowSuccessEnvelope(kind: string, value?: string | null): WorkflowResultEnvelope;
  workflowErrorEnvelope(kind: string, code: string, message: string, detail?: string | null): WorkflowResultEnvelope;
  parsePlanMarkdown(markdown?: string | null): ParsedPlan;
  extractPlanRefs(parsedPlan: ParsedPlanInput): PlanRefsResult;
  computeSyncPruneDecisions(scope?: string | null, trackedArtifacts?: string[] | null, syncedArtifacts?: string[] | null): NativeJsonValue;
  diffSnapshotManifests(oldFiles: NativeJsonValue, newFiles: NativeJsonValue, options?: ManifestIdentityOptions | null): NativeJsonValue;
  snapshotDiffFromManifests(oldFiles: NativeJsonValue, newFiles: NativeJsonValue, options?: ManifestTextDiffOptions | null): NativeJsonValue;
  bindingInfo(): LanguageBindingInfo;
  version(): string;
  projectTaskList(payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  formatTaskList(payload: NativeJsonValue, options?: TaskListOutputOptions | null): string;
  formatTaskShow(payload: NativeJsonValue): string;
  projectSnapshotList(payload: NativeJsonValue, all?: boolean | null): NativeJsonValue;
  formatSnapshotList(payload: NativeJsonValue, all?: boolean | null): string;
  inspectSnapshot(repoRoot: string, selector: string, includeDelta?: boolean | null): SnapshotInspection;
  formatSnapshotShow(inspection: SnapshotInspection, files?: boolean | null): string;
  snapshotAncestry(repoRoot: string, selector: string, options?: SnapshotAncestryOptions | null): NativeJsonValue;
  snapshotIsAncestor(repoRoot: string, older: string, newer: string): WorkflowPhaseResult;
  snapshotMergeBase(repoRoot: string, left: string, right: string, all?: boolean | null): WorkflowPhaseResult;
  formatSnapshotAncestry(payload: NativeJsonValue, all?: boolean | null): string;
  formatSnapshotIsAncestor(payload: NativeJsonValue): string;
  formatSnapshotMergeBase(payload: NativeJsonValue): string;
  formatSnapshotQueryError(message: string): string;
  snapshotQueryErrorExitCode(): number;
  formatInitializeRepository(payload: NativeJsonValue): string;
  formatConfigurationShow(payload: NativeJsonValue): string;
  formatConfiguration(payload: NativeJsonValue, requestPayload: NativeJsonValue): string;
  unsetConfiguration(repoRoot: string, key: string): NativeJsonValue;
  formatConfigurationUnset(payload: NativeJsonValue, key: string): string;
  formatRemoteAdd(payload: NativeJsonValue): string;
  formatRemoteList(payload: NativeJsonValue): string;
  queueSummary(repoRoot: string, remoteName?: string | null): NativeJsonValue;
  formatQueueSummary(payload: NativeJsonValue): string;
  doctorMemoryRoot(repoRoot: string): NativeJsonValue;
  doctorPlanAuthority(repoRoot: string): NativeJsonValue;
  formatDoctorMemoryRoot(payload: NativeJsonValue): string;
  formatDoctorRuntimeRoot(payload: NativeJsonValue): string;
  formatDoctorPlanAuthority(payload: NativeJsonValue): string;
  repositoryAuthorityRoot(repoRoot: string): string;
  formatTaskQuick(payload: NativeJsonValue): string;
  taskAudit(repoRoot: string, taskId: string, options?: ScopeOptions | null): NativeJsonValue;
  formatTaskAudit(payload: NativeJsonValue, taskId: string): string;
  taskAbandon(repoRoot: string, taskId: string, options?: ScopeOptions | null): WorkflowPhaseResult;
  formatTaskAbandon(payload: NativeJsonValue, taskId: string): string;
  formatPlanList(payload: NativeJsonValue, options?: PlanListOutputOptions | null): string;
  formatPlanRevisions(payload: NativeJsonValue, planId: string, options?: PlanListOutputOptions | null): string;
  formatPlanShow(payload: NativeJsonValue): string;
  formatPlanItems(payload: NativeJsonValue): string;
  formatPlanInspect(payload: NativeJsonValue): string;
  formatPlanCandidates(payload: NativeJsonValue): string;
  formatPlanSync(payload: NativeJsonValue): string;
  workflowReady(repoRoot: string, taskId: string, options?: WorkflowReadyOptions | null): WorkflowPhaseResult;
  workflowFinish(repoRoot: string, taskId: string, options?: WorkflowFinishOptions | null): WorkflowPhaseResult;
  workflowReconcile(repoRoot: string, options?: WorkflowReconcileOptions | null): NativeJsonValue;
  workflowGuide(topic?: string | null): NativeJsonValue;
  projectWorkflowReady(repoRoot: string, payload: NativeJsonValue, taskId: string, options?: WorkflowOutputOptions | null): NativeJsonValue;
  projectWorkflowFinish(repoRoot: string, payload: NativeJsonValue, taskId: string, options?: WorkflowOutputOptions | null): NativeJsonValue;
  formatWorkflowReady(payload: NativeJsonValue, taskId: string): string;
  formatWorkflowFinish(payload: NativeJsonValue, taskId: string): string;
  formatWorkflowReconcile(payload: NativeJsonValue): string;
  formatWorkflowGuide(payload: NativeJsonValue): string;
  planList(repoRoot: string, options?: ScopeOptions | null): NativeJsonValue[];
  planShow(repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  planRevisions(repoRoot: string, planId: string, options?: ScopeOptions | null): NativeJsonValue[];
  planItems(repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  planInspect(repoRoot: string, planId: string, options?: PlanQueryOptions | null): NativeJsonValue;
  planCandidates(repoRoot: string, options?: PlanCandidatesOptions | null): NativeJsonValue;
  planSync(repoRoot: string, target: string, options?: PlanSyncOptions | null): NativeJsonValue;
  planSyncError(payload: NativeJsonValue): string | null;
  formatStatus(payload: NativeJsonValue): string;
  formatDomainError(message: string): string;
  domainErrorExitCode(): number;
  decodeJson(text: string): NativeJsonValue;
  initializeRepository(repoRoot: string, options?: InitOptions | null): NativeJsonValue;
  configureRepository(repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  configurationError(payload: NativeJsonValue): string | null;
  taskStart(repoRoot: string, request: TaskStartRequest): NativeJsonValue;
  taskQuick(repoRoot: string, request: TaskQuickRequest): NativeJsonValue;
  taskFinish(repoRoot: string, taskId: string, options?: TaskFinishOptions | null): NativeJsonValue;
  taskFinishExitCode(payload: NativeJsonValue): number;
  snapshotCapture(repoRoot: string, reference: string, options?: SnapshotCaptureOptions | null): NativeJsonValue;
  projectTaskStart(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  formatTaskStart(payload: NativeJsonValue): string;
  projectTaskFinish(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  formatTaskFinish(payload: NativeJsonValue, requested: string): string;
  projectSnapshotCapture(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  formatSnapshotCapture(payload: NativeJsonValue): string;
  projectStatus(payload: NativeJsonValue, full?: boolean | null): NativeJsonValue;
  encodeJson(value: NativeJsonValue, pretty?: boolean | null): string;
  planPortsNormalizePlanStoreReadRequest(payloadJson: string): NativeJsonValue;
  validatePlanningSessionJoinPayload(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanRemoteTransport(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanRemoteRequest(payloadJson: string): NativeJsonValue;
  planPortsNormalizeArtifactResolverRequest(payloadJson: string): NativeJsonValue;
  planPortsNormalizeArtifactPublishRequest(payloadJson: string): NativeJsonValue;
  planPortsNormalizeLinkedTaskLookupPayload(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanConfigRuntimeFacts(payloadJson: string): NativeJsonValue;
  planPortsNormalizePlanConnectionManagerStats(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeSelectionRequest(payloadJson: string): NativeJsonValue;
  planConfigRuntimeBuildSelectionFacts(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeSelectionFacts(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeCompatibility(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeReadiness(payloadJson: string): NativeJsonValue;
  planConfigRuntimeNormalizeDoctor(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeRequest(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeBackendIdentity(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeCompatibility(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeReadiness(payloadJson: string): NativeJsonValue;
  planDiagnosticsNormalizeDoctor(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildBackendIdentityFacts(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildStorageReadinessFacts(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildCompatibilityStatus(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildReadinessStatus(payloadJson: string): NativeJsonValue;
  planDiagnosticsBuildDoctorFacts(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeListRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildList(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeShowRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildShow(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeRevisionsRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildRevisions(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeItemsRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildItems(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeCandidatesRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildCandidates(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeInspectRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildInspect(payloadJson: string): NativeJsonValue;
  planApplicationNormalizeSyncRequest(payloadJson: string): NativeJsonValue;
  planApplicationBuildSync(payloadJson: string): NativeJsonValue;
  planCommandNormalizeListRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildList(payloadJson: string): NativeJsonValue;
  planCommandNormalizeShowRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildShow(payloadJson: string): NativeJsonValue;
  planCommandNormalizeRevisionsRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildRevisions(payloadJson: string): NativeJsonValue;
  planCommandNormalizeItemsRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildItems(payloadJson: string): NativeJsonValue;
  planCommandNormalizeCandidatesRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildCandidates(payloadJson: string): NativeJsonValue;
  planCommandNormalizeInspectRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildInspect(payloadJson: string): NativeJsonValue;
  planCommandNormalizeSyncRequest(payloadJson: string): NativeJsonValue;
  planCommandBuildSync(payloadJson: string): NativeJsonValue;
  planCommandExecuteList(payloadJson: string): NativeJsonValue;
  planCommandExecuteShow(payloadJson: string): NativeJsonValue;
  planCommandExecuteRevisions(payloadJson: string): NativeJsonValue;
  planCommandExecuteItems(payloadJson: string): NativeJsonValue;
  planCommandExecuteCandidates(payloadJson: string): NativeJsonValue;
  planCommandExecuteInspect(payloadJson: string): NativeJsonValue;
  planCommandExecuteSync(payloadJson: string): NativeJsonValue;
  planProvenanceNormalizeRevisionProvenance(payloadJson: string): NativeJsonValue;
  planProvenanceBuildRevisionProvenance(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeTimestampRequest(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeTimestamp(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildTimestamp(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeSequenceIdentityRequest(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeSequenceIdentity(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildSequenceIdentity(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeWorkflowIdRequest(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeWorkflowId(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildWorkflowId(payloadJson: string): NativeJsonValue;
  planTimeIdentityNormalizeTemporalOrdering(payloadJson: string): NativeJsonValue;
  planTimeIdentityBuildTemporalOrdering(payloadJson: string): NativeJsonValue;
  taskWorkflowInit(repoRoot: string, options?: InitOptions | null): NativeJsonValue;
  taskWorkflowConfigShow(repoRoot: string): NativeJsonValue;
  taskWorkflowConfigSet(repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  taskWorkflowStatus(repoRoot: string): NativeJsonValue;
  taskWorkflowRemoteAdd(repoRoot: string, payload: NativeJsonValue): NativeJsonValue;
  taskWorkflowRemoteList(repoRoot: string): NativeJsonValue;
  taskWorkflowDoctorRuntimeRoot(repoRoot: string): NativeJsonValue;
  taskWorkflowDoctorPlanAuthority(): NativeJsonValue;
  taskWorkflowTaskStart(repoRoot: string, title: string, intent: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowTaskList(repoRoot: string, options?: TaskListOptions | null): NativeJsonValue[];
  taskWorkflowTaskShow(repoRoot: string, taskId: string, options?: ScopeOptions | null): NativeJsonValue;
  taskWorkflowSnapshotCreate(repoRoot: string, message?: string | null): NativeJsonValue;
  taskWorkflowSnapshotList(repoRoot: string): NativeJsonValue;
  taskWorkflowSnapshotShow(repoRoot: string, snapshotId: string): NativeJsonValue;
  /** @deprecated Throws the native chat retirement error. */
  agentCapabilities(): AgentCapabilitiesPayload;
  /** @deprecated Throws the native chat retirement error. */
  agentManagement<T extends JsonValue = JsonValue>(request: JsonObject): T;
  /** @deprecated Throws the native chat retirement error. */
  agentWorkerTransaction<T extends JsonValue = JsonValue>(
    request: JsonObject,
  ): T;
}

export function requiredAddonExports(): string[];

/** @deprecated Historical payload shape; current native calls throw a retirement error. */
export interface AgentCapabilitiesPayload extends JsonObject {
  contract: "ait.agent.worker.capabilities.v1";
  binary: "ait-agent-worker";
  version: string;
  platform: string;
  architecture: string;
  supported_transports: AgentTransport[];
  event_loop_backends: string[];
  default_event_loop_backend: string;
  python_worker_execution_allowed: false;
}

/** @deprecated Legacy payload parser; no current chat capability is advertised. */
export class AgentCapabilities {
  constructor(payload: AgentCapabilitiesPayload);
  readonly contract: "ait.agent.worker.capabilities.v1";
  readonly version: string;
  readonly platform: string;
  readonly architecture: string;
  readonly supportedTransports: readonly AgentTransport[];
  readonly eventLoopBackends: readonly string[];
  readonly defaultEventLoopBackend: string;
  readonly raw: AgentCapabilitiesPayload;
}

export interface AgentContext {
  cwd?: string | URL;
  repoRoot?: string | URL;
  manifestPath?: string | URL;
  env?: EnvironmentOverrides;
}

export interface AgentManagementContext extends AgentContext {
  workerBinary?: string | URL;
}

export interface AgentLogsContext extends AgentManagementContext {
  lines?: number;
}

export interface AgentWorkerContext extends AgentContext {
  worker?: string;
  signature?: string;
  signatureTimestamp?: string;
  nowUnixSeconds?: number;
}

/** @deprecated Chat integrations are retired in AIT 1.2.0. Calls throw native errors. */
export class AgentClient {
  constructor(runtime?: NativeRuntime);
  readonly runtime: NativeRuntime;
  capabilities(): AgentCapabilities;
  manage<T extends JsonValue = JsonValue>(request: JsonObject): T;
  add<T extends JsonValue = JsonValue>(
    worker: JsonObject,
    options?: AgentManagementContext,
  ): T;
  listWorkers<T extends JsonValue = JsonValue>(
    transport: AgentTransport,
    options?: AgentManagementContext,
  ): T;
  status<T extends JsonValue = JsonValue>(
    transport: AgentTransport,
    name?: string | null,
    options?: AgentManagementContext,
  ): T;
  start<T extends JsonValue = JsonValue>(
    transport: AgentTransport,
    name: string,
    options?: AgentManagementContext,
  ): T;
  stop<T extends JsonValue = JsonValue>(
    transport: AgentTransport,
    name: string,
    options?: AgentManagementContext,
  ): T;
  restart<T extends JsonValue = JsonValue>(
    transport: AgentTransport,
    name: string,
    options?: AgentManagementContext,
  ): T;
  remove<T extends JsonValue = JsonValue>(
    transport: AgentTransport,
    name: string,
    options?: AgentManagementContext,
  ): T;
  logs<T extends JsonValue = JsonValue>(
    transport: AgentTransport,
    name: string,
    options?: AgentLogsContext,
  ): T;
  workerTransaction<T extends JsonValue = JsonValue>(
    operation: AgentWorkerOperation,
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
  slackCommand<T extends JsonValue = JsonValue>(
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
  discordInteraction<T extends JsonValue = JsonValue>(
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
  replyProvider<T extends JsonValue = JsonValue>(
    payload: JsonValue,
    options?: AgentWorkerContext,
  ): T;
}
