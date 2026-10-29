import { lstatSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { LANGUAGE_BINDING_CONTRACT } from "./contract.js";
import {
  NativeProtocolError,
  NativeResolutionError,
} from "./errors.js";

export const DEFAULT_REQUEST_TIMEOUT_SECONDS = 30;
export const LONG_RUNNING_REQUEST_TIMEOUT_SECONDS = 1800;

export function requiredAddonConstants() {
  return { DEFAULT_REQUEST_TIMEOUT_SECONDS, LONG_RUNNING_REQUEST_TIMEOUT_SECONDS };
}

const REQUIRED_STRING_CONSTANTS = Object.freeze(["CODE_REVIEW_SUMMARY_TEMPLATE", "CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE", "CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND", "DEFAULT_ID_NAMESPACE_PREFIX", "LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX", "REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX", "RUNTIME_DATA_ENV", "LEGACY_SERVER_DATA_ENV", "CONTRACT_VERSION", "PLAN_CONTRACT_VERSION", "TARGET_TRIPLE"]);

export function requiredAddonStringConstants() {
  return [...REQUIRED_STRING_CONSTANTS];
}

const require = createRequire(import.meta.url);
const CONTRACT_PATH = fileURLToPath(
  new URL("../lib/npm-payload-contract.json", import.meta.url),
);
const LOCAL_ADDON_PATH = fileURLToPath(
  new URL("../native/ait_napi.node", import.meta.url),
);
const REQUIRED_EXPORTS = Object.freeze([
  "bindingInfoJson",
  "taskWorkflowInit",
  "taskWorkflowConfigShow",
  "taskWorkflowConfigSet",
  "taskWorkflowStatus",
  "taskWorkflowRemoteAdd",
  "taskWorkflowRemoteList",
  "taskWorkflowDoctorRuntimeRoot",
  "taskWorkflowDoctorPlanAuthority",
  "taskWorkflowTaskStart",
  "taskWorkflowTaskList",
  "taskWorkflowTaskShow",
  "taskWorkflowSnapshotCreate",
  "taskWorkflowSnapshotList",
  "taskWorkflowSnapshotShow",
  "planPortsNormalizePlanStoreReadRequest",
  "validatePlanningSessionJoinPayload",
  "planPortsNormalizePlanRemoteTransport",
  "planPortsNormalizePlanRemoteRequest",
  "planPortsNormalizeArtifactResolverRequest",
  "planPortsNormalizeArtifactPublishRequest",
  "planPortsNormalizeLinkedTaskLookupPayload",
  "planPortsNormalizePlanConfigRuntimeFacts",
  "planPortsNormalizePlanConnectionManagerStats",
  "planConfigRuntimeNormalizeSelectionRequest",
  "planConfigRuntimeBuildSelectionFacts",
  "planConfigRuntimeNormalizeSelectionFacts",
  "planConfigRuntimeNormalizeCompatibility",
  "planConfigRuntimeNormalizeReadiness",
  "planConfigRuntimeNormalizeDoctor",
  "planDiagnosticsNormalizeRequest",
  "planDiagnosticsNormalizeBackendIdentity",
  "planDiagnosticsNormalizeCompatibility",
  "planDiagnosticsNormalizeReadiness",
  "planDiagnosticsNormalizeDoctor",
  "planDiagnosticsBuildBackendIdentityFacts",
  "planDiagnosticsBuildStorageReadinessFacts",
  "planDiagnosticsBuildCompatibilityStatus",
  "planDiagnosticsBuildReadinessStatus",
  "planDiagnosticsBuildDoctorFacts",
  "planApplicationNormalizeListRequest",
  "planApplicationBuildList",
  "planApplicationNormalizeShowRequest",
  "planApplicationBuildShow",
  "planApplicationNormalizeRevisionsRequest",
  "planApplicationBuildRevisions",
  "planApplicationNormalizeItemsRequest",
  "planApplicationBuildItems",
  "planApplicationNormalizeCandidatesRequest",
  "planApplicationBuildCandidates",
  "planApplicationNormalizeInspectRequest",
  "planApplicationBuildInspect",
  "planApplicationNormalizeSyncRequest",
  "planApplicationBuildSync",
  "planCommandNormalizeListRequest",
  "planCommandBuildList",
  "planCommandNormalizeShowRequest",
  "planCommandBuildShow",
  "planCommandNormalizeRevisionsRequest",
  "planCommandBuildRevisions",
  "planCommandNormalizeItemsRequest",
  "planCommandBuildItems",
  "planCommandNormalizeCandidatesRequest",
  "planCommandBuildCandidates",
  "planCommandNormalizeInspectRequest",
  "planCommandBuildInspect",
  "planCommandNormalizeSyncRequest",
  "planCommandBuildSync",
  "planCommandExecuteList",
  "planCommandExecuteShow",
  "planCommandExecuteRevisions",
  "planCommandExecuteItems",
  "planCommandExecuteCandidates",
  "planCommandExecuteInspect",
  "planCommandExecuteSync",
  "planProvenanceNormalizeRevisionProvenance",
  "planProvenanceBuildRevisionProvenance",
  "planTimeIdentityNormalizeTimestampRequest",
  "planTimeIdentityNormalizeTimestamp",
  "planTimeIdentityBuildTimestamp",
  "planTimeIdentityNormalizeSequenceIdentityRequest",
  "planTimeIdentityNormalizeSequenceIdentity",
  "planTimeIdentityBuildSequenceIdentity",
  "planTimeIdentityNormalizeWorkflowIdRequest",
  "planTimeIdentityNormalizeWorkflowId",
  "planTimeIdentityBuildWorkflowId",
  "planTimeIdentityNormalizeTemporalOrdering",
  "planTimeIdentityBuildTemporalOrdering",
  "initializeRepository",
  "configureRepository",
  "configurationError",
  "taskStart",
  "taskQuick",
  "taskFinish",
  "taskFinishExitCode",
  "snapshotCapture",
  "projectTaskStart",
  "formatTaskStart",
  "projectTaskFinish",
  "formatTaskFinish",
  "projectSnapshotCapture",
  "formatSnapshotCapture",
  "projectStatus",
  "encodeJson",
  "formatStatus",
  "formatDomainError",
  "domainErrorExitCode",
  "decodeJson",
  "planList",
  "planShow",
  "planRevisions",
  "planItems",
  "planInspect",
  "planCandidates",
  "planSync",
  "planSyncError",
  "formatPlanList",
  "formatPlanRevisions",
  "formatPlanShow",
  "formatPlanItems",
  "formatPlanInspect",
  "formatPlanCandidates",
  "formatPlanSync",
  "workflowReady",
  "workflowFinish",
  "workflowReconcile",
  "workflowGuide",
  "projectWorkflowReady",
  "projectWorkflowFinish",
  "formatWorkflowReady",
  "formatWorkflowFinish",
  "formatWorkflowReconcile",
  "formatWorkflowGuide",
  "repositoryAuthorityRoot",
  "formatTaskQuick",
  "taskAudit",
  "formatTaskAudit",
  "taskAbandon",
  "formatTaskAbandon",
  "formatInitializeRepository",
  "formatConfigurationShow",
  "formatConfiguration",
  "unsetConfiguration",
  "formatConfigurationUnset",
  "formatRemoteAdd",
  "formatRemoteList",
  "queueSummary",
  "formatQueueSummary",
  "doctorMemoryRoot",
  "doctorPlanAuthority",
  "formatDoctorMemoryRoot",
  "formatDoctorRuntimeRoot",
  "formatDoctorPlanAuthority",
  "snapshotAncestry",
  "snapshotIsAncestor",
  "snapshotMergeBase",
  "formatSnapshotAncestry",
  "formatSnapshotIsAncestor",
  "formatSnapshotMergeBase",
  "formatSnapshotQueryError",
  "snapshotQueryErrorExitCode",
  "projectTaskList",
  "formatTaskList",
  "formatTaskShow",
  "projectSnapshotList",
  "formatSnapshotList",
  "inspectSnapshot",
  "formatSnapshotShow",
  "taskWorkflowLineList",
  "lineInventory",
  "taskWorkflowLineCreate",
  "taskWorkflowLineSwitch",
  "taskWorkflowLineShow",
  "lineShow",
  "taskWorkflowLineArchive",
  "taskWorkflowLineRename",
  "taskWorkflowLineDelete",
  "taskWorkflowLineSetHead",
  "lineMerge",
  "taskWorkflowLineCleanup",
  "formatLineList",
  "formatLineCreate",
  "formatLineSwitch",
  "formatLineShow",
  "formatLineArchive",
  "formatLineRename",
  "formatLineDelete",
  "formatLineMerge",
  "formatLineCleanup",
  "repositoryShow",
  "repositoryJobs",
  "repositoryCiCapabilities",
  "repositoryRetire",
  "repositoryRestore",
  "formatRepositoryShow",
  "formatRepositoryJobs",
  "formatRepositoryCiCapabilities",
  "formatRepositoryRetire",
  "formatRepositoryRestore",
  "taskWorkflowChangeList",
  "taskWorkflowChangeShow",
  "taskWorkflowChangeRevert",
  "taskWorkflowChangeReplay",
  "taskWorkflowChangeClose",
  "taskWorkflowChangePublish",
  "taskWorkflowPatchsetPublish",
  "taskWorkflowPatchsetList",
  "taskWorkflowPatchsetShow",
  "taskWorkflowPatchsetSelect",
  "taskWorkflowPatchsetPublishExplicit",
  "taskWorkflowPatchsetCiStatus",
  "taskWorkflowPatchsetRerunCi",
  "taskWorkflowReviewTeamApprove",
  "taskWorkflowReviewShow",
  "taskWorkflowReviewRequest",
  "taskWorkflowReviewTaskApprove",
  "taskWorkflowReviewRecord",
  "taskWorkflowReviewCodeSubmit",
  "taskWorkflowReviewCodeTemplate",
  "taskWorkflowAttestPut",
  "taskWorkflowAttestShow",
  "taskWorkflowPolicyEval",
  "taskWorkflowPolicyShow",
  "taskWorkflowPolicyWaive",
  "changeCreate",
  "changeShow",
  "changeRevert",
  "changeReplay",
  "changeClose",
  "changePublish",
  "patchsetPublish",
  "patchsetList",
  "formatChangeCreate",
  "formatChangeShow",
  "formatChangeRevert",
  "formatChangeReplay",
  "formatChangeClose",
  "formatChangePublish",
  "formatPatchsetPublish",
  "formatPatchsetList",
  "formatPatchsetShow",
  "formatPatchsetSelect",
  "formatPatchsetCiStatus",
  "formatPatchsetRerunCi",
  "projectChangeList",
  "formatChangeList",
  "WorkspaceCommandLock",
  "workspaceCommandLockPath",
  "taskWorkflowSnapshotChain",
  "taskWorkflowSnapshotExists",
  "taskWorkflowBlobReadBytes",
  "taskWorkflowBlobEnsureBytes",
  "taskWorkflowWorkspaceDelta",
  "taskWorkflowWorkspaceRestore",
  "taskWorkflowWorkspaceRestorePaths",
  "taskWorkflowSnapshotDiff",
  "taskWorkflowSnapshotRevert",
  "taskWorkflowSnapshotReplay",
  "taskWorkflowSnapshotCreateExplicit",
  "taskWorkflowSnapshotRestoreLines",
  "snapshotDiff",
  "snapshotRevert",
  "snapshotReplay",
  "formatSnapshotDiff",
  "formatSnapshotRevert",
  "formatSnapshotReplay",
  "formatSnapshotRestoreLines",
  "taskWorkflowWorktreeGet",
  "taskWorkflowWorktreeStatus",
  "taskWorkflowWorktreeRestore",
  "taskWorkflowWorktreeList",
  "taskWorkflowWorktreeTouchUsage",
  "taskWorkflowWorktreeDoctor",
  "taskWorkflowWorktreeCleanupCandidates",
  "taskWorkflowWorktreeCleanup",
  "taskWorkflowWorktreePruneStale",
  "taskWorkflowWorktreeRemove",
  "taskWorkflowWorktreeBindExisting",
  "taskWorkflowWorktreeSync",
  "taskWorkflowWorktreeSyncAll",
  "taskWorkflowWorktreeRecreate",
  "taskWorkflowWorktreeRestoreOwnedHead",
  "taskWorkflowWorktreePreviewRebase",
  "taskWorkflowWorktreeRebase",
  "taskWorkflowWorktreeContinueRebase",
  "taskWorkflowWorktreeAbortRebase",
  "taskWorkflowAuthWhoami",
  "taskWorkflowAuthBindings",
  "taskWorkflowAuthGrant",
  "formatAuthWhoami",
  "formatAuthBindings",
  "formatAuthGrant",
  "stashSave",
  "taskWorkflowStashList",
  "taskWorkflowStashShow",
  "stashApply",
  "stashPop",
  "stashDrop",
  "taskWorkflowStashDrop",
  "formatStashSave",
  "formatStashList",
  "formatStashShow",
  "formatStashApply",
  "formatStashPop",
  "formatStashDrop",
  "tagCreate",
  "tagList",
  "tagShow",
  "tagDelete",
  "resolveSnapshotRef",
  "formatTagCreate",
  "formatTagList",
  "formatTagShow",
  "formatTagDelete",
  "reviewShow",
  "reviewTeamRequest",
  "reviewTeamApprove",
  "reviewTeamRequestChanges",
  "reviewTeamComment",
  "reviewTeamDefer",
  "reviewTaskApprove",
  "reviewTaskRequestChanges",
  "reviewTaskComment",
  "reviewTaskDefer",
  "reviewCodeSubmit",
  "formatReviewTaskApprove",
  "formatReviewTaskRequestChanges",
  "formatReviewTaskComment",
  "formatReviewTaskDefer",
  "formatReviewTeamRequest",
  "formatReviewTeamApprove",
  "formatReviewTeamRequestChanges",
  "formatReviewTeamComment",
  "formatReviewTeamDefer",
  "formatReviewShow",
  "formatReviewCodeSubmit",
  "formatReviewCodeTemplate",
  "projectReviewTaskOperation",
  "projectReviewShow",
  "validateReviewPatchsetSelector",
  "validatePublicPatchsetArgument",
  "formatAttestPut",
  "formatAttestShow",
  "formatPolicyEval",
  "formatPolicyShow",
  "formatPolicyWaive",
  "projectPolicyShow",
  "workspaceDiff",
  "renderWorkspaceDiffNames",
  "renderWorkspaceDiffStat",
  "renderWorkspaceDiffText",
  "taskWorkflowBlame",
  "formatBlame",
  "pullRepository",
  "pushRepository",
  "taskWorkflowPull",
  "taskWorkflowPush",
  "taskWorkflowUploadSnapshotChain",
  "formatPull",
  "formatPush",
  "projectRemoteSync",
  "worktreeShow",
  "worktreePath",
  "worktreeRestore",
  "worktreeCleanup",
  "worktreePruneStale",
  "worktreeSync",
  "worktreeRecreate",
  "worktreeRecoverTask",
  "worktreeRestoreOwnedHead",
  "worktreeRebase",
  "worktreeRemove",
  "formatWorktreeStatus",
  "formatWorktreePath",
  "formatWorktreeShow",
  "formatWorktreeList",
  "formatWorktreeDoctor",
  "formatWorktreeCleanupCandidates",
  "formatWorktreeCleanup",
  "formatWorktreePruneStale",
  "formatWorktreeSync",
  "formatWorktreeRestore",
  "formatWorktreeRecreate",
  "formatWorktreeRecoverTask",
  "formatWorktreeRestoreOwnedHead",
  "formatWorktreeRebase",
  "formatWorktreeRemove",
  "projectWorktreePath",
  "worktreeSyncExitCode",
  "gitMigrate",
  "gitRecover",
  "gitImport",
  "gitExport",
  "gitRestore",
  "gitMirror",
  "formatGitMigrate",
  "formatGitRecover",
  "formatGitImport",
  "formatGitExport",
  "formatGitRestore",
  "formatGitMirror",
  "gcStats",
  "gcValidate",
  "gcPrune",
  "gcValidateExitCode",
  "formatGcStats",
  "formatGcValidate",
  "formatGcPrune",
  "policyProfileNames",
  "authorModeValues",
  "policyContentClassValues",
  "policyAuthorClassValues",
  "normalizeAuthorMode",
  "missingCodeReviewSummarySections",
  "isStructuredCodeReviewSummary",
  "renderCodeReviewSummaryTemplate",
  "codeReviewSummaryRequirementText",
  "derivePolicyContentClass",
  "derivePolicyAuthorClass",
  "buildMinimumProvenance",
  "policyProfile",
  "normalizePolicy",
  "resolveEffectivePolicy",
  "policyToYaml",
  "parsePolicyYaml",
  "runtimeDataEnvValue",
  "runtimeDataEnvName",
  "resolveRuntimeDataRootWithSource",
  "resolveRuntimeDataRoot",
  "resolveServerRuntimeRootWithSource",
  "resolveServerRuntimeRoot",
  "encodeRefName",
  "decodeRefName",
  "normalizeTaskStatus",
  "normalizeOptionalText",
  "utcNow",
  "generateNamespacedWorkflowId",
  "normalizeIdNamespacePrefix",
  "workflowIdTokens",
  "workflowOriginNamespacePrefix",
  "workflowIdNamespacePrefixCandidates",
  "workflowIdNamespacePrefixForValue",
  "workflowIdMatchesAnyNamespacePrefix",
  "derivePatchsetId",
  "generateWorkflowId",
  "taskStatusDetails",
  "normalizePublicationState",
  "publicationStateHasUnpublishedHead",
  "workflowIdToken",
  "workflowIdMatches",
  "generateNamespacedSequenceId",
  "workflowSuccessEnvelope",
  "workflowErrorEnvelope",
  "parsePlanMarkdown",
  "extractPlanRefs",
  "computeSyncPruneDecisions",
  "diffSnapshotManifests",
  "snapshotDiffFromManifests",
  "currentCoreSourceFingerprint",
  "currentServerSourceFingerprint",
  "currentSourceNativeCacheContract",
  "currentSourceCacheMarkBuilding",
  "currentSourceCacheActivate",
  "currentSourceCacheReleaseLease",
  "currentSourceCachePrune",
  "currentSourceCacheSeedCanonical",
  "currentSourceCacheExtensionFresh",
  "currentSourceCacheBinaryFresh",
  "currentSourceCacheCoreFingerprint",
  "currentSourceCacheServerFingerprint",
  "currentSourceCacheCoreMtime",
  "currentSourceCacheServerMtime",
  "formatCurrentSourceCacheContract",
  "formatCurrentSourceCacheMarkBuilding",
  "formatCurrentSourceCacheActivate",
  "formatCurrentSourceCacheReleaseLease",
  "formatCurrentSourceCachePrune",
  "formatCurrentSourceCacheSeedCanonical",
  "formatCurrentSourceCacheExtensionFresh",
  "formatCurrentSourceCacheBinaryFresh",
  "formatCurrentSourceCacheCoreFingerprint",
  "formatCurrentSourceCacheServerFingerprint",
  "formatCurrentSourceCacheCoreMtime",
  "formatCurrentSourceCacheServerMtime",
  "extractPlanItems",
  "listPlanSectionRefs",
  "extractPlanSection",
  "findPlanItem",
  "normalizePlanItems",
  "findPlanItemInItems",
  "normalizeTaskWorkflowHttpCompatibility",
  "normalizeTaskWorkflowHttpReadiness",
  "planItemsPayload",
  "localPlanPublishShadow",
  "planDispatchSummary",
  "computeTaskableItems",
  "validateDispatchLegality",
  "planTaskLinkIndexes",
  "planCandidatesPayload",
  "artifactBlobId",
  "artifactCandidatesOpen",
  "indexPlansByArtifactPath",
  "indexPlansByArtifactIdentity",
  "planArtifactIdentity",
  "planArtifactIdentityLabel",
  "openGenericPlansMatchingBlobId",
  "openPlansMatchingSelector",
  "localPlanFullyPublished",
  "planHeadsEquivalent",
  "planMatchesSyncArtifact",
  "planFilesystemNormalizeMarkdownArtifactPath",
  "planFilesystemIsMarkdownArtifactPath",
  "planFilesystemIsLineageOnlyMarkdownArtifactPath",
  "planFilesystemReadUtf8TextFile",
  "planFilesystemReadJsonFile",
  "planFilesystemReadBinaryFile",
  "planFilesystemPathIsProjectedOutForWorkspace",
  "planFilesystemWorkspacePathIsIgnored",
  "planFilesystemListVisibleWorkspacePaths",
  "planFilesystemListVisibleMarkdownArtifactPaths",
  "planFilesystemZipArchiveHasMember",
  "planFilesystemReadZipArchiveMember",
  "planFilesystemResolveRepoArtifactPath",
  "repoStateReadRepoConfigJson",
  "repoStateWriteRepoConfigJson",
  "repoStateReadWorktreeConfigJson",
  "repoStateWriteWorktreeConfigJson",
  "repoStateReadWorktreeMetadataJson",
  "repoStateWriteWorktreeMetadataJson",
  "planPackSubstrateBuildPackMembers",
  "planPackSubstrateBuildTreePackMembers",
  "planPackSubstrateBuildGitBinaryDelta",
  "planPackSubstrateBuildGitBinaryDeltaMember",
  "planPackSubstrateApplyGitBinaryDelta",
  "planPackSubstrateApplyPackDelta",
  "planPackSubstrateWritePackArchive",
  "planPackSubstrateWriteTreePackArchive",
  "planPackSubstrateReadPackIndex",
  "planPackSubstrateReadTreePackIndex",
  "planPackSubstratePackHasEntry",
  "planPackSubstrateReadTreePackTree",
  "planPackSubstrateSummarizePackArchives",
  "planPackSubstrateSummarizeTreePackArchives",
  "planPackSubstrateTreePackManifestPath",
  "planPackSubstrateReadTreePackTreeByOrdinal",
  "planPackSubstrateBuildStorageValidationSummary",
  "planPackSubstrateBuildTreeRecords",
  "planPackSubstrateBuildSnapshotId",
  "joinPath",
  "buildQueryString",
  "normalizeRemoteText",
  "buildListPlansRequest",
  "buildGetPlanRequest",
  "buildListPlanRevisionsRequest",
  "buildGetPlanRevisionRequest",
  "buildCreatePlanRequest",
  "buildRevisePlanRequest",
  "buildUpdatePlanStatusRequest",
  "buildPutPlanRevisionArtifactsRequest",
  "listPlans",
  "getPlan",
  "listPlanRevisions",
  "getPlanRevision",
  "resolveTaskPlanLinkage",
  "listPlanIdsMatchingContains",
  "createPlan",
  "revisePlan",
  "updatePlanStatus",
  "putPlanRevisionArtifacts",
  "createPlanningSession",
  "listPlanningSessions",
  "getPlanningSession",
  "appendPlanningSessionEvent",
  "listPlanningSessionEvents",
  "joinPlanningSession",
  "promotePlanningSession",
  "closePlanningSession",
  "PlanRemoteTransportConfig",
  "PlanHttpClientManager",
  "TaskWorkflowHttpClientManager",
  "HttpTaskRemote",
  "HttpWorkflowCloseoutRemote",
  "extractCodexUsageJsonl",
  "extractCodexUsageBundleJsonl",
  "projectRepoRetireRuntimeBlockers",
  "languageBindingInfo",
  "transportEnvelopeIrVersion",
  "transportEnvelopeSchema",
  "runtimeBindingStateIrVersion",
  "runtimeBindingStateSchema",
  "runtimeBindingStateDefaultPayload",
  "runtimeBindingStateNormalizeDocument",
  "workerManifestIrVersion",
  "workerManifestSchema",
  "workerManifestDefaultConfig",
  "workerManifestNormalizeDocument",
  "workerManifestSelectTelegramWorker",
  "transportEnvelopeBuildBindingMetadata",
  "transportEnvelopeBuildEventEnvelope",
  "transportEnvelopeBuildReplyEnvelope",
  "transportEnvelopeCompactEventEnvelope",
  "transportEnvelopeCompactReplyEnvelope",
  "taskWorkflowTimestampFacts",
  "taskWorkflowSequenceIdentityFacts",
  "taskWorkflowWorkflowIdFacts",
  "taskWorkflowRuntimeSelectionFacts",
  "taskWorkflowReadUtf8TextFile",
  "taskWorkflowReadJsonFile",
  "taskWorkflowReadBinaryFile",
  "taskWorkflowResolveRepoArtifactPath",
  "taskRemoteChangeLineagePayload",
  "taskWorkflowResolveTaskWorktreeLocation",
  "taskWorkflowResolveMainSeedMirrorLocation",
  "taskWorkflowEnsureMainSeedMirror",
  "taskWorkflowEnsureStatusManifest",
  "taskWorkflowQueueSummary",
  "taskWorkflowTaskAudit",
  "taskWorkflowTaskAbandon",
  "taskWorkflowTaskLand",
  "taskWorkflowLandSubmit",
  "taskWorkflowLandShow",
  "taskWorkflowLandRetry",
  "buildLinkedTaskLookupPayload",
  "buildLinkedChangeLookupPayload",
  "buildTaskTrackingTitlePayload",
  "buildTaskTrackingMetadataPayload",
  "buildTaskAuditVerdictPayload",
  "workflowLandedFacts",
  "workflowLandFullFacts",
  "workflowReadyFacts",
  "workflowLandPhaseFacts",
  "projectWorkflowLandPhaseReadModel",
  "workflowRemoteMutationReceipt",
  "workflowRemoteActionMutationReceipts",
  "workflowApplyPhasePayload",
  "workflowApplyPhaseSummary",
  "workflowMutationReceiptSummary",
  "workflowAppliedActionSummary",
  "workflowReadyPayloadDirect",
  "workflowLandPayloadDirect",
  "workflowReadyApplyDirect",
  "workflowLandApplyDirect",
  "projectWorkflowLandedReadModel",
  "projectWorkflowLandFullReadModel",
  "projectWorkflowReadyReadModel",
  "createWorkflowRelease",
  "listWorkflowReleases",
  "getWorkflowRelease",
  "updateWorkflowRelease",
  "taskWorkflowReleaseCandidateCreate",
  "taskWorkflowReleaseCheck",
  "taskWorkflowReleaseBuild",
  "taskWorkflowReleaseFormula",
  "taskWorkflowReleaseShow",
  "taskWorkflowReleasePublish",
  "releaseAdapterCheck",
  "releaseAdapterBuild",
  "releaseCandidateCreate",
  "releaseCheck",
  "releaseBuild",
  "releaseNativeSource",
  "releaseNativeBundle",
  "releasePackage",
  "releaseFormula",
  "releaseShow",
  "releasePublish",
  "releasePromote",
  "formatRelease",
  "externalStatus",
  "externalDoctor",
  "externalUpdate",
  "externalLink",
  "externalUnlink",
  "teamContacts",
  "teamSessionEnsure",
  "teamSessionAcknowledge",
  "formatTeamContacts",
  "formatTeamSessionEnsure",
  "formatTeamSessionAcknowledge",
  "binaryDbUpgradeU64Seconds",
  "binaryDbAgentPool50",
  "formatBinaryDbUpgradeU64Seconds",
  "formatBinaryDbAgentPool50",
  "formatExternalStatus",
  "formatExternalDoctor",
  "formatExternalUpdate",
  "formatExternalLink",
  "formatExternalUnlink",
  "remoteRecoverHead",
  "formatRemoteRecoverHead",
  "parseSnapshotAuthoringReference",
  "workflowIdFamilies",
  "workflowTaskChangeOriginNamespacePrefixes",
  "policyRequirementFlags",
  // Retained compatibility symbols. Presence checks must never invoke them.
  "agentWorkerCapabilitiesJson",
  "agentManagementJson",
  "agentWorkerTransactionJson",
]);
const CONTRACT_KEYS = Object.freeze([
  "family_version",
  "payloads",
  "schema",
  "top_level_package",
]);
const PAYLOAD_KEYS = Object.freeze([
  "addon",
  "binding_repository",
  "binding_snapshot",
  "component",
  "cpu",
  "libc",
  "license",
  "os",
  "package",
  "target",
  "version",
]);
const ADDON_METADATA_KEYS = Object.freeze([
  "addon",
  "binding_repository",
  "binding_snapshot",
  "component",
  "libc",
  "schema",
  "target",
]);
const TARGETS = new Map([
  ["aarch64-apple-darwin", { os: "darwin", cpu: "arm64", libc: null }],
  ["x86_64-apple-darwin", { os: "darwin", cpu: "x64", libc: null }],
  ["aarch64-unknown-linux-gnu", { os: "linux", cpu: "arm64", libc: "glibc" }],
  ["x86_64-unknown-linux-gnu", { os: "linux", cpu: "x64", libc: "glibc" }],
  ["aarch64-pc-windows-msvc", { os: "win32", cpu: "arm64", libc: null }],
  ["x86_64-pc-windows-msvc", { os: "win32", cpu: "x64", libc: null }],
]);

let validatedContract = null;

export class WorkspaceCommandLock {
  #lock;

  constructor(repoRoot, commandName, runtime = new NativeRuntime()) {
    const NativeLock = runtime.loadAddon().WorkspaceCommandLock;
    this.#lock = new NativeLock(repoRoot, commandName);
  }

  metadata() {
    return this.#lock.metadata();
  }

  close() {
    return this.#lock.close();
  }

  release() {
    return this.#lock.release();
  }

  isClosed() {
    return this.#lock.isClosed();
  }
}

export class PlanRemoteTransportConfig {
  #config;

  constructor(requestTimeoutSeconds, longRunningRequestTimeoutSeconds, runtime = new NativeRuntime()) {
    const NativeConfig = runtime.loadAddon().PlanRemoteTransportConfig;
    this.#config = new NativeConfig(requestTimeoutSeconds, longRunningRequestTimeoutSeconds);
  }

  requestTimeoutSeconds(longRunning) {
    return this.#config.requestTimeoutSeconds(longRunning);
  }
}

export class PlanHttpClientManager {
  #manager;

  constructor(baseUrl, options, runtime = new NativeRuntime()) {
    const NativeManager = runtime.loadAddon().PlanHttpClientManager;
    this.#manager = new NativeManager(baseUrl, options);
  }

  inspect() {
    return this.#manager.inspect();
  }

  close() {
    return this.#manager.close();
  }

  listPlans(repoName, artifactPath) {
    return this.#manager.listPlans(repoName, artifactPath);
  }

  getPlan(planId) {
    return this.#manager.getPlan(planId);
  }

  listPlanRevisions(planId) {
    return this.#manager.listPlanRevisions(planId);
  }

  getPlanRevision(planId, planRevisionId) {
    return this.#manager.getPlanRevision(planId, planRevisionId);
  }

  createPlan(input) {
    return this.#manager.createPlan(input);
  }

  revisePlan(input) {
    return this.#manager.revisePlan(input);
  }

  updatePlanStatus(planId, status) {
    return this.#manager.updatePlanStatus(planId, status);
  }

  putPlanRevisionArtifacts(planId, planRevisionId, artifacts) {
    return this.#manager.putPlanRevisionArtifacts(planId, planRevisionId, artifacts);
  }

  createPlanningSession(planId, input) {
    return this.#manager.createPlanningSession(planId, input);
  }

  listPlanningSessions(planId, status) {
    return this.#manager.listPlanningSessions(planId, status);
  }

  getPlanningSession(planningSessionId) {
    return this.#manager.getPlanningSession(planningSessionId);
  }

  appendPlanningSessionEvent(planningSessionId, eventType, payload) {
    return this.#manager.appendPlanningSessionEvent(planningSessionId, eventType, payload);
  }

  listPlanningSessionEvents(planningSessionId, input) {
    return this.#manager.listPlanningSessionEvents(planningSessionId, input);
  }

  joinPlanningSession(planningSessionId, input) {
    return this.#manager.joinPlanningSession(planningSessionId, input);
  }

  promotePlanningSession(planningSessionId, input) {
    return this.#manager.promotePlanningSession(planningSessionId, input);
  }

  closePlanningSession(planningSessionId, status) {
    return this.#manager.closePlanningSession(planningSessionId, status);
  }

}

export class TaskWorkflowHttpClientManager {
  #manager;

  constructor(baseUrl, options, runtime = new NativeRuntime()) {
    const NativeManager = runtime.loadAddon().TaskWorkflowHttpClientManager;
    this.#manager = new NativeManager(baseUrl, options);
  }

  inspect() {
    return this.#manager.inspect();
  }

  close() {
    return this.#manager.close();
  }
}

export class HttpTaskRemote {
  #adapter;

  constructor(baseUrl, options, runtime = new NativeRuntime()) {
    const NativeAdapter = runtime.loadAddon().HttpTaskRemote;
    this.#adapter = new NativeAdapter(baseUrl, options);
  }

  inspectClient() {
    return this.#adapter.inspectClient();
  }

  closeClient() {
    return this.#adapter.closeClient();
  }

  changeLineagePayload(baseLine, lineRow) {
    return this.#adapter.changeLineagePayload(baseLine, lineRow);
  }

  getLine(repoName, lineName) {
    return this.#adapter.getLine(repoName, lineName);
  }

  getTask(taskId, repoName) {
    return this.#adapter.getTask(taskId, repoName);
  }

  listTasks(repoName) {
    return this.#adapter.listTasks(repoName);
  }

  createTask(repoName, title, intent, options) {
    return this.#adapter.createTask(repoName, title, intent, options);
  }

  listChanges(repoName) {
    return this.#adapter.listChanges(repoName);
  }

  getChangeDetail(changeRef, repoName) {
    return this.#adapter.getChangeDetail(changeRef, repoName);
  }

  getChange(changeRef, repoName) {
    return this.#adapter.getChange(changeRef, repoName);
  }

  closeChange(changeRef, status, repoName) {
    return this.#adapter.closeChange(changeRef, status, repoName);
  }

  createChange(repoName, taskId, title, baseLine, options) {
    return this.#adapter.createChange(repoName, taskId, title, baseLine, options);
  }

  updateRemoteLine(repoName, lineName, options) {
    return this.#adapter.updateRemoteLine(repoName, lineName, options);
  }

  planRemoteZstdBulk(repoName, request) {
    return this.#adapter.planRemoteZstdBulk(repoName, request);
  }

  putRemoteZstdObjectPack(repoName, packId, packBytes) {
    return this.#adapter.putRemoteZstdObjectPack(repoName, packId, packBytes);
  }

  putRemoteZstdTreePack(repoName, packId, packBytes) {
    return this.#adapter.putRemoteZstdTreePack(repoName, packId, packBytes);
  }

  commitRemoteZstdBulk(repoName, request) {
    return this.#adapter.commitRemoteZstdBulk(repoName, request);
  }

  getRemoteZstdImportManifest(repoName, snapshotId) {
    return this.#adapter.getRemoteZstdImportManifest(repoName, snapshotId);
  }

  getRemoteZstdObjectPack(repoName, packId) {
    return this.#adapter.getRemoteZstdObjectPack(repoName, packId);
  }

  getRemoteZstdTreePack(repoName, packId) {
    return this.#adapter.getRemoteZstdTreePack(repoName, packId);
  }

  getRemoteSnapshot(repoName, snapshotId, options) {
    return this.#adapter.getRemoteSnapshot(repoName, snapshotId, options);
  }

  getRemoteSnapshotsExistence(repoName, snapshotIds) {
    return this.#adapter.getRemoteSnapshotsExistence(repoName, snapshotIds);
  }

}

export class HttpWorkflowCloseoutRemote {
  #adapter;

  constructor(baseUrl, options, runtime = new NativeRuntime()) {
    const NativeAdapter = runtime.loadAddon().HttpWorkflowCloseoutRemote;
    this.#adapter = new NativeAdapter(baseUrl, options);
  }

  inspectClient() {
    return this.#adapter.inspectClient();
  }

  closeClient() {
    return this.#adapter.closeClient();
  }

  mutationReceipt(input) {
    return this.#adapter.mutationReceipt(input);
  }

  actionMutationReceipts(code, result) {
    return this.#adapter.actionMutationReceipts(code, result);
  }

  listPatchsets(changeId, repoName) {
    return this.#adapter.listPatchsets(changeId, repoName);
  }

  getPatchset(patchsetId, options) {
    return this.#adapter.getPatchset(patchsetId, options);
  }

  publishPatchset(input) {
    return this.#adapter.publishPatchset(input);
  }

  selectPatchset(changeId, patchsetId, options) {
    return this.#adapter.selectPatchset(changeId, patchsetId, options);
  }

  runPatchsetCi(patchsetId, options) {
    return this.#adapter.runPatchsetCi(patchsetId, options);
  }

  readPatchsetCiStatus(patchsetId, options) {
    return this.#adapter.readPatchsetCiStatus(patchsetId, options);
  }

  requestReview(input) {
    return this.#adapter.requestReview(input);
  }

  listReviews(changeId, options) {
    return this.#adapter.listReviews(changeId, options);
  }

  recordReview(input) {
    return this.#adapter.recordReview(input);
  }

  putAttestation(patchsetId, authorMode, statement, options) {
    return this.#adapter.putAttestation(patchsetId, authorMode, statement, options);
  }

  getAttestation(patchsetId, options) {
    return this.#adapter.getAttestation(patchsetId, options);
  }

  evaluatePolicy(patchsetId, options) {
    return this.#adapter.evaluatePolicy(patchsetId, options);
  }

  getPolicy(patchsetId, options) {
    return this.#adapter.getPolicy(patchsetId, options);
  }

  createWaiver(input) {
    return this.#adapter.createWaiver(input);
  }

  submitLand(input) {
    return this.#adapter.submitLand(input);
  }

  getLand(submissionId, repoName) {
    return this.#adapter.getLand(submissionId, repoName);
  }

  retryLand(submissionId, reason, repoName) {
    return this.#adapter.retryLand(submissionId, reason, repoName);
  }

  closeTask(taskId, options) {
    return this.#adapter.closeTask(taskId, options);
  }

}

export class NativeRuntime {
  get CODE_REVIEW_SUMMARY_TEMPLATE() {
    return this.loadAddon().CODE_REVIEW_SUMMARY_TEMPLATE;
  }

  get CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE() {
    return this.loadAddon().CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE;
  }

  get CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND() {
    return this.loadAddon().CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND;
  }

  get DEFAULT_ID_NAMESPACE_PREFIX() {
    return this.loadAddon().DEFAULT_ID_NAMESPACE_PREFIX;
  }

  get LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX() {
    return this.loadAddon().LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX;
  }

  get REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX() {
    return this.loadAddon().REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX;
  }

  get RUNTIME_DATA_ENV() {
    return this.loadAddon().RUNTIME_DATA_ENV;
  }

  get LEGACY_SERVER_DATA_ENV() {
    return this.loadAddon().LEGACY_SERVER_DATA_ENV;
  }

  get CONTRACT_VERSION() {
    return this.loadAddon().CONTRACT_VERSION;
  }

  get PLAN_CONTRACT_VERSION() {
    return this.loadAddon().PLAN_CONTRACT_VERSION;
  }

  get TARGET_TRIPLE() {
    return this.loadAddon().TARGET_TRIPLE;
  }

  get DEFAULT_REQUEST_TIMEOUT_SECONDS() {
    return this.loadAddon().DEFAULT_REQUEST_TIMEOUT_SECONDS;
  }

  get LONG_RUNNING_REQUEST_TIMEOUT_SECONDS() {
    return this.loadAddon().LONG_RUNNING_REQUEST_TIMEOUT_SECONDS;
  }


  constructor(options = {}) {
    if (!isRecord(options)) {
      throw new TypeError("NativeRuntime options must be an object");
    }
    const unknown = Object.keys(options).filter((name) => name !== "addonPath");
    if (unknown.length > 0) {
      throw new TypeError(
        `unsupported NativeRuntime options: ${unknown.sort().join(", ")}`,
      );
    }
    this.addonPath =
      options.addonPath === undefined
        ? null
        : normalizeAddonPath(options.addonPath);
    this.addon = null;
  }

  resolveAddonPath() {
    if (this.addonPath !== null) {
      assertRegularAddon(this.addonPath);
      return this.addonPath;
    }

    if (regularNonSymlinkFile(LOCAL_ADDON_PATH)) {
      this.addonPath = LOCAL_ADDON_PATH;
      return this.addonPath;
    }

    const payload = selectPlatformPayload(loadContract());
    let packageJsonPath;
    try {
      packageJsonPath = require.resolve(`${payload.package}/package.json`);
    } catch (error) {
      if (error?.code === "MODULE_NOT_FOUND") {
        throw new NativeResolutionError(
          `Rust AIT binding package ${payload.package}@${payload.version} is missing; reinstall with optional dependencies enabled`,
          { cause: error },
        );
      }
      throw error;
    }
    const packageJson = readJson(packageJsonPath, payload.package);
    validatePlatformPackage(packageJson, payload);
    const packageRoot = path.dirname(packageJsonPath);
    const addonPath = path.resolve(packageRoot, ...payload.addon.split("/"));
    if (
      addonPath === packageRoot ||
      !addonPath.startsWith(`${packageRoot}${path.sep}`)
    ) {
      throw new NativeResolutionError(
        `Rust AIT binding path for ${payload.package} escapes its package`,
      );
    }
    assertRegularAddon(addonPath);
    this.addonPath = addonPath;
    return addonPath;
  }

  loadAddon() {
    if (this.addon !== null) {
      return this.addon;
    }
    const addonPath = this.resolveAddonPath();
    let addon;
    try {
      addon = require(addonPath);
    } catch (error) {
      throw new NativeResolutionError(
        `Rust AIT binding could not be loaded from ${addonPath}`,
        { cause: error },
      );
    }
    if (!isRecord(addon)) {
      throw new NativeResolutionError(
        `Rust AIT binding did not export an object: ${addonPath}`,
      );
    }
    if (typeof addon.runCli === "function") {
      throw new NativeResolutionError(
        "Rust AIT binding exposes the removed CLI relay; use direct function bindings",
      );
    }
    for (const name of REQUIRED_EXPORTS) {
      if (typeof addon[name] !== "function") {
        throw new NativeResolutionError(
          `Rust AIT binding does not export ${name} as a supported function`,
        );
      }
    }
    for (const [name, value] of Object.entries(addon)) {
      if (typeof value === "function" && !REQUIRED_EXPORTS.includes(name)) {
        throw new NativeResolutionError(
          `Rust AIT binding exposes an unmapped function: ${name}`,
        );
      }
    }
    for (const [name, expected] of Object.entries(requiredAddonConstants())) {
      if (addon[name] !== expected) {
        throw new NativeResolutionError(`Rust AIT binding has incompatible constant ${name}`);
      }
    }
    for (const name of REQUIRED_STRING_CONSTANTS) {
      if (typeof addon[name] !== "string") {
        throw new NativeResolutionError(`Rust AIT binding does not export ${name} as a string constant`);
      }
    }
    this.addon = addon;
    return addon;
  }

  resolveCallable(name) {
    if (typeof name !== "string" || name.trim().length === 0) {
      throw new NativeResolutionError(
        "native addon export name must be non-empty text",
      );
    }
    const exportName = name.trim();
    if (!REQUIRED_EXPORTS.includes(exportName)) {
      throw new NativeResolutionError(
        `Rust AIT binding does not declare ${exportName} as a supported function`,
      );
    }
    const resolved = this.loadAddon()[exportName];
    if (typeof resolved !== "function") {
      throw new NativeResolutionError(
        `Rust AIT binding does not export ${exportName} as a supported function`,
      );
    }
    return resolved;
  }

  call(name, ...args) {
    return this.resolveCallable(name)(...args);
  }

  bindingInfo() {
    const payload = parseObjectPayload(
      this.call("bindingInfoJson"),
      "language binding info",
    );
    if (payload.contract !== LANGUAGE_BINDING_CONTRACT) {
      throw new NativeProtocolError(
        "ait-napi returned an unsupported language binding contract",
      );
    }
    if (payload.runtime_authority !== "rust") {
      throw new NativeProtocolError(
        "ait-napi language binding does not identify Rust authority",
      );
    }
    if (payload.node_binding !== "napi") {
      throw new NativeProtocolError(
        "ait-napi language binding does not identify Node-API",
      );
    }
    if (payload.process_transport_allowed !== false) {
      throw new NativeProtocolError(
        "ait-napi language binding permits a process API transport",
      );
    }
    if (
      !Array.isArray(payload.supported_surfaces) ||
      payload.supported_surfaces.length !== 1 ||
      payload.supported_surfaces[0] !== "ait-core"
    ) {
      throw new NativeProtocolError(
        "ait-napi language binding must support only the retained ait-core surface",
      );
    }
    const version = requiredText(payload, "version", "language binding info");
    if (version !== loadContract().family_version) {
      throw new NativeProtocolError(
        `ait-napi version ${version} does not match npm family ${loadContract().family_version}`,
      );
    }
    return payload;
  }

  version() {
    return requiredText(this.bindingInfo(), "version", "language binding info");
  }

  taskWorkflowInit(repoRoot, options) {
    return this.call("taskWorkflowInit", repoRoot, options);
  }

  taskWorkflowConfigShow(repoRoot) {
    return this.call("taskWorkflowConfigShow", repoRoot);
  }

  taskWorkflowConfigSet(repoRoot, payload) {
    return this.call("taskWorkflowConfigSet", repoRoot, payload);
  }

  taskWorkflowStatus(repoRoot) {
    return this.call("taskWorkflowStatus", repoRoot);
  }

  taskWorkflowRemoteAdd(repoRoot, payload) {
    return this.call("taskWorkflowRemoteAdd", repoRoot, payload);
  }

  taskWorkflowRemoteList(repoRoot) {
    return this.call("taskWorkflowRemoteList", repoRoot);
  }

  taskWorkflowDoctorRuntimeRoot(repoRoot) {
    return this.call("taskWorkflowDoctorRuntimeRoot", repoRoot);
  }

  taskWorkflowDoctorPlanAuthority() {
    return this.call("taskWorkflowDoctorPlanAuthority");
  }

  taskWorkflowTaskStart(repoRoot, title, intent, options) {
    return this.call("taskWorkflowTaskStart", repoRoot, title, intent, options);
  }

  taskWorkflowTaskList(repoRoot, options) {
    return this.call("taskWorkflowTaskList", repoRoot, options);
  }

  taskWorkflowTaskShow(repoRoot, taskId, options) {
    return this.call("taskWorkflowTaskShow", repoRoot, taskId, options);
  }

  taskWorkflowSnapshotCreate(repoRoot, message) {
    return this.call("taskWorkflowSnapshotCreate", repoRoot, message);
  }

  taskWorkflowSnapshotList(repoRoot) {
    return this.call("taskWorkflowSnapshotList", repoRoot);
  }

  taskWorkflowSnapshotShow(repoRoot, snapshotId) {
    return this.call("taskWorkflowSnapshotShow", repoRoot, snapshotId);
  }

  planPortsNormalizePlanStoreReadRequest(payloadJson) {
    return this.call("planPortsNormalizePlanStoreReadRequest", payloadJson);
  }

  validatePlanningSessionJoinPayload(payloadJson) {
    return this.call("validatePlanningSessionJoinPayload", payloadJson);
  }

  planPortsNormalizePlanRemoteTransport(payloadJson) {
    return this.call("planPortsNormalizePlanRemoteTransport", payloadJson);
  }

  planPortsNormalizePlanRemoteRequest(payloadJson) {
    return this.call("planPortsNormalizePlanRemoteRequest", payloadJson);
  }

  planPortsNormalizeArtifactResolverRequest(payloadJson) {
    return this.call("planPortsNormalizeArtifactResolverRequest", payloadJson);
  }

  planPortsNormalizeArtifactPublishRequest(payloadJson) {
    return this.call("planPortsNormalizeArtifactPublishRequest", payloadJson);
  }

  planPortsNormalizeLinkedTaskLookupPayload(payloadJson) {
    return this.call("planPortsNormalizeLinkedTaskLookupPayload", payloadJson);
  }

  planPortsNormalizePlanConfigRuntimeFacts(payloadJson) {
    return this.call("planPortsNormalizePlanConfigRuntimeFacts", payloadJson);
  }

  planPortsNormalizePlanConnectionManagerStats(payloadJson) {
    return this.call("planPortsNormalizePlanConnectionManagerStats", payloadJson);
  }

  planConfigRuntimeNormalizeSelectionRequest(payloadJson) {
    return this.call("planConfigRuntimeNormalizeSelectionRequest", payloadJson);
  }

  planConfigRuntimeBuildSelectionFacts(payloadJson) {
    return this.call("planConfigRuntimeBuildSelectionFacts", payloadJson);
  }

  planConfigRuntimeNormalizeSelectionFacts(payloadJson) {
    return this.call("planConfigRuntimeNormalizeSelectionFacts", payloadJson);
  }

  planConfigRuntimeNormalizeCompatibility(payloadJson) {
    return this.call("planConfigRuntimeNormalizeCompatibility", payloadJson);
  }

  planConfigRuntimeNormalizeReadiness(payloadJson) {
    return this.call("planConfigRuntimeNormalizeReadiness", payloadJson);
  }

  planConfigRuntimeNormalizeDoctor(payloadJson) {
    return this.call("planConfigRuntimeNormalizeDoctor", payloadJson);
  }

  planDiagnosticsNormalizeRequest(payloadJson) {
    return this.call("planDiagnosticsNormalizeRequest", payloadJson);
  }

  planDiagnosticsNormalizeBackendIdentity(payloadJson) {
    return this.call("planDiagnosticsNormalizeBackendIdentity", payloadJson);
  }

  planDiagnosticsNormalizeCompatibility(payloadJson) {
    return this.call("planDiagnosticsNormalizeCompatibility", payloadJson);
  }

  planDiagnosticsNormalizeReadiness(payloadJson) {
    return this.call("planDiagnosticsNormalizeReadiness", payloadJson);
  }

  planDiagnosticsNormalizeDoctor(payloadJson) {
    return this.call("planDiagnosticsNormalizeDoctor", payloadJson);
  }

  planDiagnosticsBuildBackendIdentityFacts(payloadJson) {
    return this.call("planDiagnosticsBuildBackendIdentityFacts", payloadJson);
  }

  planDiagnosticsBuildStorageReadinessFacts(payloadJson) {
    return this.call("planDiagnosticsBuildStorageReadinessFacts", payloadJson);
  }

  planDiagnosticsBuildCompatibilityStatus(payloadJson) {
    return this.call("planDiagnosticsBuildCompatibilityStatus", payloadJson);
  }

  planDiagnosticsBuildReadinessStatus(payloadJson) {
    return this.call("planDiagnosticsBuildReadinessStatus", payloadJson);
  }

  planDiagnosticsBuildDoctorFacts(payloadJson) {
    return this.call("planDiagnosticsBuildDoctorFacts", payloadJson);
  }

  planApplicationNormalizeListRequest(payloadJson) {
    return this.call("planApplicationNormalizeListRequest", payloadJson);
  }

  planApplicationBuildList(payloadJson) {
    return this.call("planApplicationBuildList", payloadJson);
  }

  planApplicationNormalizeShowRequest(payloadJson) {
    return this.call("planApplicationNormalizeShowRequest", payloadJson);
  }

  planApplicationBuildShow(payloadJson) {
    return this.call("planApplicationBuildShow", payloadJson);
  }

  planApplicationNormalizeRevisionsRequest(payloadJson) {
    return this.call("planApplicationNormalizeRevisionsRequest", payloadJson);
  }

  planApplicationBuildRevisions(payloadJson) {
    return this.call("planApplicationBuildRevisions", payloadJson);
  }

  planApplicationNormalizeItemsRequest(payloadJson) {
    return this.call("planApplicationNormalizeItemsRequest", payloadJson);
  }

  planApplicationBuildItems(payloadJson) {
    return this.call("planApplicationBuildItems", payloadJson);
  }

  planApplicationNormalizeCandidatesRequest(payloadJson) {
    return this.call("planApplicationNormalizeCandidatesRequest", payloadJson);
  }

  planApplicationBuildCandidates(payloadJson) {
    return this.call("planApplicationBuildCandidates", payloadJson);
  }

  planApplicationNormalizeInspectRequest(payloadJson) {
    return this.call("planApplicationNormalizeInspectRequest", payloadJson);
  }

  planApplicationBuildInspect(payloadJson) {
    return this.call("planApplicationBuildInspect", payloadJson);
  }

  planApplicationNormalizeSyncRequest(payloadJson) {
    return this.call("planApplicationNormalizeSyncRequest", payloadJson);
  }

  planApplicationBuildSync(payloadJson) {
    return this.call("planApplicationBuildSync", payloadJson);
  }

  planCommandNormalizeListRequest(payloadJson) {
    return this.call("planCommandNormalizeListRequest", payloadJson);
  }

  planCommandBuildList(payloadJson) {
    return this.call("planCommandBuildList", payloadJson);
  }

  planCommandNormalizeShowRequest(payloadJson) {
    return this.call("planCommandNormalizeShowRequest", payloadJson);
  }

  planCommandBuildShow(payloadJson) {
    return this.call("planCommandBuildShow", payloadJson);
  }

  planCommandNormalizeRevisionsRequest(payloadJson) {
    return this.call("planCommandNormalizeRevisionsRequest", payloadJson);
  }

  planCommandBuildRevisions(payloadJson) {
    return this.call("planCommandBuildRevisions", payloadJson);
  }

  planCommandNormalizeItemsRequest(payloadJson) {
    return this.call("planCommandNormalizeItemsRequest", payloadJson);
  }

  planCommandBuildItems(payloadJson) {
    return this.call("planCommandBuildItems", payloadJson);
  }

  planCommandNormalizeCandidatesRequest(payloadJson) {
    return this.call("planCommandNormalizeCandidatesRequest", payloadJson);
  }

  planCommandBuildCandidates(payloadJson) {
    return this.call("planCommandBuildCandidates", payloadJson);
  }

  planCommandNormalizeInspectRequest(payloadJson) {
    return this.call("planCommandNormalizeInspectRequest", payloadJson);
  }

  planCommandBuildInspect(payloadJson) {
    return this.call("planCommandBuildInspect", payloadJson);
  }

  planCommandNormalizeSyncRequest(payloadJson) {
    return this.call("planCommandNormalizeSyncRequest", payloadJson);
  }

  planCommandBuildSync(payloadJson) {
    return this.call("planCommandBuildSync", payloadJson);
  }

  planCommandExecuteList(payloadJson) {
    return this.call("planCommandExecuteList", payloadJson);
  }

  planCommandExecuteShow(payloadJson) {
    return this.call("planCommandExecuteShow", payloadJson);
  }

  planCommandExecuteRevisions(payloadJson) {
    return this.call("planCommandExecuteRevisions", payloadJson);
  }

  planCommandExecuteItems(payloadJson) {
    return this.call("planCommandExecuteItems", payloadJson);
  }

  planCommandExecuteCandidates(payloadJson) {
    return this.call("planCommandExecuteCandidates", payloadJson);
  }

  planCommandExecuteInspect(payloadJson) {
    return this.call("planCommandExecuteInspect", payloadJson);
  }

  planCommandExecuteSync(payloadJson) {
    return this.call("planCommandExecuteSync", payloadJson);
  }

  planProvenanceNormalizeRevisionProvenance(payloadJson) {
    return this.call("planProvenanceNormalizeRevisionProvenance", payloadJson);
  }

  planProvenanceBuildRevisionProvenance(payloadJson) {
    return this.call("planProvenanceBuildRevisionProvenance", payloadJson);
  }

  planTimeIdentityNormalizeTimestampRequest(payloadJson) {
    return this.call("planTimeIdentityNormalizeTimestampRequest", payloadJson);
  }

  planTimeIdentityNormalizeTimestamp(payloadJson) {
    return this.call("planTimeIdentityNormalizeTimestamp", payloadJson);
  }

  planTimeIdentityBuildTimestamp(payloadJson) {
    return this.call("planTimeIdentityBuildTimestamp", payloadJson);
  }

  planTimeIdentityNormalizeSequenceIdentityRequest(payloadJson) {
    return this.call("planTimeIdentityNormalizeSequenceIdentityRequest", payloadJson);
  }

  planTimeIdentityNormalizeSequenceIdentity(payloadJson) {
    return this.call("planTimeIdentityNormalizeSequenceIdentity", payloadJson);
  }

  planTimeIdentityBuildSequenceIdentity(payloadJson) {
    return this.call("planTimeIdentityBuildSequenceIdentity", payloadJson);
  }

  planTimeIdentityNormalizeWorkflowIdRequest(payloadJson) {
    return this.call("planTimeIdentityNormalizeWorkflowIdRequest", payloadJson);
  }

  planTimeIdentityNormalizeWorkflowId(payloadJson) {
    return this.call("planTimeIdentityNormalizeWorkflowId", payloadJson);
  }

  planTimeIdentityBuildWorkflowId(payloadJson) {
    return this.call("planTimeIdentityBuildWorkflowId", payloadJson);
  }

  planTimeIdentityNormalizeTemporalOrdering(payloadJson) {
    return this.call("planTimeIdentityNormalizeTemporalOrdering", payloadJson);
  }

  planTimeIdentityBuildTemporalOrdering(payloadJson) {
    return this.call("planTimeIdentityBuildTemporalOrdering", payloadJson);
  }

  initializeRepository(repoRoot, options) {
    return this.call("initializeRepository", repoRoot, options);
  }

  configureRepository(repoRoot, payload) {
    return this.call("configureRepository", repoRoot, payload);
  }

  configurationError(payload) {
    return this.call("configurationError", payload);
  }

  taskStart(repoRoot, request) {
    return this.call("taskStart", repoRoot, request);
  }

  taskQuick(repoRoot, request) {
    return this.call("taskQuick", repoRoot, request);
  }

  taskFinish(repoRoot, taskId, options) {
    return this.call("taskFinish", repoRoot, taskId, options);
  }

  taskFinishExitCode(payload) {
    return this.call("taskFinishExitCode", payload);
  }

  snapshotCapture(repoRoot, reference, options) {
    return this.call("snapshotCapture", repoRoot, reference, options);
  }

  projectTaskStart(payload, full) {
    return this.call("projectTaskStart", payload, full);
  }

  formatTaskStart(payload) {
    return this.call("formatTaskStart", payload);
  }

  projectTaskFinish(payload, full) {
    return this.call("projectTaskFinish", payload, full);
  }

  formatTaskFinish(payload, requested) {
    return this.call("formatTaskFinish", payload, requested);
  }

  projectSnapshotCapture(payload, full) {
    return this.call("projectSnapshotCapture", payload, full);
  }

  formatSnapshotCapture(payload) {
    return this.call("formatSnapshotCapture", payload);
  }

  projectStatus(payload, full) {
    return this.call("projectStatus", payload, full);
  }

  encodeJson(value, pretty) {
    return this.call("encodeJson", value, pretty);
  }

  formatStatus(payload) {
    return this.call("formatStatus", payload);
  }

  formatDomainError(message) {
    return this.call("formatDomainError", message);
  }

  domainErrorExitCode() {
    return this.call("domainErrorExitCode");
  }

  decodeJson(text) {
    return this.call("decodeJson", text);
  }

  planList(repoRoot, options) {
    return this.call("planList", repoRoot, options);
  }

  planShow(repoRoot, planId, options) {
    return this.call("planShow", repoRoot, planId, options);
  }

  planRevisions(repoRoot, planId, options) {
    return this.call("planRevisions", repoRoot, planId, options);
  }

  planItems(repoRoot, planId, options) {
    return this.call("planItems", repoRoot, planId, options);
  }

  planInspect(repoRoot, planId, options) {
    return this.call("planInspect", repoRoot, planId, options);
  }

  planCandidates(repoRoot, options) {
    return this.call("planCandidates", repoRoot, options);
  }

  planSync(repoRoot, target, options) {
    return this.call("planSync", repoRoot, target, options);
  }

  planSyncError(payload) {
    return this.call("planSyncError", payload);
  }

  formatPlanList(payload, options) {
    return this.call("formatPlanList", payload, options);
  }

  formatPlanRevisions(payload, planId, options) {
    return this.call("formatPlanRevisions", payload, planId, options);
  }

  formatPlanShow(payload) {
    return this.call("formatPlanShow", payload);
  }

  formatPlanItems(payload) {
    return this.call("formatPlanItems", payload);
  }

  formatPlanInspect(payload) {
    return this.call("formatPlanInspect", payload);
  }

  formatPlanCandidates(payload) {
    return this.call("formatPlanCandidates", payload);
  }

  formatPlanSync(payload) {
    return this.call("formatPlanSync", payload);
  }

  workflowReady(repoRoot, taskId, options) {
    return this.call("workflowReady", repoRoot, taskId, options);
  }

  workflowFinish(repoRoot, taskId, options) {
    return this.call("workflowFinish", repoRoot, taskId, options);
  }

  workflowReconcile(repoRoot, options) {
    return this.call("workflowReconcile", repoRoot, options);
  }

  workflowGuide(topic) {
    return this.call("workflowGuide", topic);
  }

  projectWorkflowReady(repoRoot, payload, taskId, options) {
    return this.call("projectWorkflowReady", repoRoot, payload, taskId, options);
  }

  projectWorkflowFinish(repoRoot, payload, taskId, options) {
    return this.call("projectWorkflowFinish", repoRoot, payload, taskId, options);
  }

  formatWorkflowReady(payload, taskId) {
    return this.call("formatWorkflowReady", payload, taskId);
  }

  formatWorkflowFinish(payload, taskId) {
    return this.call("formatWorkflowFinish", payload, taskId);
  }

  formatWorkflowReconcile(payload) {
    return this.call("formatWorkflowReconcile", payload);
  }

  formatWorkflowGuide(payload) {
    return this.call("formatWorkflowGuide", payload);
  }

  repositoryAuthorityRoot(repoRoot) {
    return this.call("repositoryAuthorityRoot", repoRoot);
  }

  formatTaskQuick(payload) {
    return this.call("formatTaskQuick", payload);
  }

  taskAudit(repoRoot, taskId, options) {
    return this.call("taskAudit", repoRoot, taskId, options);
  }

  formatTaskAudit(payload, taskId) {
    return this.call("formatTaskAudit", payload, taskId);
  }

  taskAbandon(repoRoot, taskId, options) {
    return this.call("taskAbandon", repoRoot, taskId, options);
  }

  formatTaskAbandon(payload, taskId) {
    return this.call("formatTaskAbandon", payload, taskId);
  }

  formatInitializeRepository(payload) {
    return this.call("formatInitializeRepository", payload);
  }

  formatConfigurationShow(payload) {
    return this.call("formatConfigurationShow", payload);
  }

  formatConfiguration(payload, requestPayload) {
    return this.call("formatConfiguration", payload, requestPayload);
  }

  unsetConfiguration(repoRoot, key) {
    return this.call("unsetConfiguration", repoRoot, key);
  }

  formatConfigurationUnset(payload, key) {
    return this.call("formatConfigurationUnset", payload, key);
  }

  formatRemoteAdd(payload) {
    return this.call("formatRemoteAdd", payload);
  }

  formatRemoteList(payload) {
    return this.call("formatRemoteList", payload);
  }

  queueSummary(repoRoot, remoteName) {
    return this.call("queueSummary", repoRoot, remoteName);
  }

  formatQueueSummary(payload) {
    return this.call("formatQueueSummary", payload);
  }

  doctorMemoryRoot(repoRoot) {
    return this.call("doctorMemoryRoot", repoRoot);
  }

  doctorPlanAuthority(repoRoot) {
    return this.call("doctorPlanAuthority", repoRoot);
  }

  formatDoctorMemoryRoot(payload) {
    return this.call("formatDoctorMemoryRoot", payload);
  }

  formatDoctorRuntimeRoot(payload) {
    return this.call("formatDoctorRuntimeRoot", payload);
  }

  formatDoctorPlanAuthority(payload) {
    return this.call("formatDoctorPlanAuthority", payload);
  }

  snapshotAncestry(repoRoot, selector, options) {
    return this.call("snapshotAncestry", repoRoot, selector, options);
  }

  snapshotIsAncestor(repoRoot, older, newer) {
    return this.call("snapshotIsAncestor", repoRoot, older, newer);
  }

  snapshotMergeBase(repoRoot, left, right, all) {
    return this.call("snapshotMergeBase", repoRoot, left, right, all);
  }

  formatSnapshotAncestry(payload, all) {
    return this.call("formatSnapshotAncestry", payload, all);
  }

  formatSnapshotIsAncestor(payload) {
    return this.call("formatSnapshotIsAncestor", payload);
  }

  formatSnapshotMergeBase(payload) {
    return this.call("formatSnapshotMergeBase", payload);
  }

  formatSnapshotQueryError(message) {
    return this.call("formatSnapshotQueryError", message);
  }

  snapshotQueryErrorExitCode() {
    return this.call("snapshotQueryErrorExitCode");
  }

  projectTaskList(payload, all) {
    return this.call("projectTaskList", payload, all);
  }

  formatTaskList(payload, options) {
    return this.call("formatTaskList", payload, options);
  }

  formatTaskShow(payload) {
    return this.call("formatTaskShow", payload);
  }

  projectSnapshotList(payload, all) {
    return this.call("projectSnapshotList", payload, all);
  }

  formatSnapshotList(payload, all) {
    return this.call("formatSnapshotList", payload, all);
  }

  inspectSnapshot(repoRoot, selector, includeDelta) {
    return this.call("inspectSnapshot", repoRoot, selector, includeDelta);
  }

  formatSnapshotShow(inspection, files) {
    return this.call("formatSnapshotShow", inspection, files);
  }

  taskWorkflowLineList(repoRoot, options) {
    return this.call("taskWorkflowLineList", repoRoot, options);
  }

  lineInventory(repoRoot, options) {
    return this.call("lineInventory", repoRoot, options);
  }

  taskWorkflowLineCreate(repoRoot, name, options) {
    return this.call("taskWorkflowLineCreate", repoRoot, name, options);
  }

  taskWorkflowLineSwitch(repoRoot, name, options) {
    return this.call("taskWorkflowLineSwitch", repoRoot, name, options);
  }

  taskWorkflowLineShow(repoRoot, name) {
    return this.call("taskWorkflowLineShow", repoRoot, name);
  }

  lineShow(repoRoot, name, remoteName) {
    return this.call("lineShow", repoRoot, name, remoteName);
  }

  taskWorkflowLineArchive(repoRoot, name, remoteName) {
    return this.call("taskWorkflowLineArchive", repoRoot, name, remoteName);
  }

  taskWorkflowLineRename(repoRoot, oldName, newName, remoteName) {
    return this.call("taskWorkflowLineRename", repoRoot, oldName, newName, remoteName);
  }

  taskWorkflowLineDelete(repoRoot, name, options) {
    return this.call("taskWorkflowLineDelete", repoRoot, name, options);
  }

  taskWorkflowLineSetHead(repoRoot, name, snapshotId) {
    return this.call("taskWorkflowLineSetHead", repoRoot, name, snapshotId);
  }

  lineMerge(repoRoot, options) {
    return this.call("lineMerge", repoRoot, options);
  }

  taskWorkflowLineCleanup(repoRoot, options) {
    return this.call("taskWorkflowLineCleanup", repoRoot, options);
  }

  formatLineList(inventory, options) {
    return this.call("formatLineList", inventory, options);
  }

  formatLineCreate(payload) {
    return this.call("formatLineCreate", payload);
  }

  formatLineSwitch(payload) {
    return this.call("formatLineSwitch", payload);
  }

  formatLineShow(payload) {
    return this.call("formatLineShow", payload);
  }

  formatLineArchive(payload) {
    return this.call("formatLineArchive", payload);
  }

  formatLineRename(payload) {
    return this.call("formatLineRename", payload);
  }

  formatLineDelete(payload) {
    return this.call("formatLineDelete", payload);
  }

  formatLineMerge(payload) {
    return this.call("formatLineMerge", payload);
  }

  formatLineCleanup(payload, idleFor, options) {
    return this.call("formatLineCleanup", payload, idleFor, options);
  }

  repositoryShow(repoRoot, remoteName) {
    return this.call("repositoryShow", repoRoot, remoteName);
  }

  repositoryJobs(repoRoot, options) {
    return this.call("repositoryJobs", repoRoot, options);
  }

  repositoryCiCapabilities(repoRoot, remoteName) {
    return this.call("repositoryCiCapabilities", repoRoot, remoteName);
  }

  repositoryRetire(repoRoot, remoteName, abort) {
    return this.call("repositoryRetire", repoRoot, remoteName, abort);
  }

  repositoryRestore(repoRoot, remoteName) {
    return this.call("repositoryRestore", repoRoot, remoteName);
  }

  formatRepositoryShow(payload, remoteName) {
    return this.call("formatRepositoryShow", payload, remoteName);
  }

  formatRepositoryJobs(payload, options) {
    return this.call("formatRepositoryJobs", payload, options);
  }

  formatRepositoryCiCapabilities(payload, remoteName) {
    return this.call("formatRepositoryCiCapabilities", payload, remoteName);
  }

  formatRepositoryRetire(payload) {
    return this.call("formatRepositoryRetire", payload);
  }

  formatRepositoryRestore(payload) {
    return this.call("formatRepositoryRestore", payload);
  }

  taskWorkflowChangeList(root, options) {
    return this.call("taskWorkflowChangeList", root, options);
  }

  taskWorkflowChangeShow(root, changeId, options) {
    return this.call("taskWorkflowChangeShow", root, changeId, options);
  }

  taskWorkflowChangeRevert(root, changeId, options) {
    return this.call("taskWorkflowChangeRevert", root, changeId, options);
  }

  taskWorkflowChangeReplay(root, changeId, onto, options) {
    return this.call("taskWorkflowChangeReplay", root, changeId, onto, options);
  }

  taskWorkflowChangeClose(root, changeId, options) {
    return this.call("taskWorkflowChangeClose", root, changeId, options);
  }

  taskWorkflowChangePublish(root, changeId, options) {
    return this.call("taskWorkflowChangePublish", root, changeId, options);
  }

  taskWorkflowPatchsetPublish(root, changeId, summary, options) {
    return this.call("taskWorkflowPatchsetPublish", root, changeId, summary, options);
  }

  taskWorkflowPatchsetList(root, changeId, options) {
    return this.call("taskWorkflowPatchsetList", root, changeId, options);
  }

  taskWorkflowPatchsetShow(root, patchsetId, options) {
    return this.call("taskWorkflowPatchsetShow", root, patchsetId, options);
  }

  taskWorkflowPatchsetSelect(root, patchsetId, options) {
    return this.call("taskWorkflowPatchsetSelect", root, patchsetId, options);
  }

  taskWorkflowPatchsetPublishExplicit(root, changeId, baseSnapshotId, revisionSnapshotId, summary, options) {
    return this.call("taskWorkflowPatchsetPublishExplicit", root, changeId, baseSnapshotId, revisionSnapshotId, summary, options);
  }

  taskWorkflowPatchsetCiStatus(root, patchsetId, options) {
    return this.call("taskWorkflowPatchsetCiStatus", root, patchsetId, options);
  }

  taskWorkflowPatchsetRerunCi(root, patchsetId, options) {
    return this.call("taskWorkflowPatchsetRerunCi", root, patchsetId, options);
  }

  taskWorkflowReviewTeamApprove(root, changeId, options) {
    return this.call("taskWorkflowReviewTeamApprove", root, changeId, options);
  }

  taskWorkflowReviewShow(root, changeId, options) {
    return this.call("taskWorkflowReviewShow", root, changeId, options);
  }

  taskWorkflowReviewRequest(root, changeId, reviewerGroups, options) {
    return this.call("taskWorkflowReviewRequest", root, changeId, reviewerGroups, options);
  }

  taskWorkflowReviewTaskApprove(root, changeId, patchsetId, message, options) {
    return this.call("taskWorkflowReviewTaskApprove", root, changeId, patchsetId, message, options);
  }

  taskWorkflowReviewRecord(root, changeId, action, options) {
    return this.call("taskWorkflowReviewRecord", root, changeId, action, options);
  }

  taskWorkflowReviewCodeSubmit(root, changeId, patchsetId, message, options) {
    return this.call("taskWorkflowReviewCodeSubmit", root, changeId, patchsetId, message, options);
  }

  taskWorkflowReviewCodeTemplate(options) {
    return this.call("taskWorkflowReviewCodeTemplate", options);
  }

  taskWorkflowAttestPut(root, options) {
    return this.call("taskWorkflowAttestPut", root, options);
  }

  taskWorkflowAttestShow(root, patchsetId, options) {
    return this.call("taskWorkflowAttestShow", root, patchsetId, options);
  }

  taskWorkflowPolicyEval(root, patchsetId, options) {
    return this.call("taskWorkflowPolicyEval", root, patchsetId, options);
  }

  taskWorkflowPolicyShow(root, patchsetId, options) {
    return this.call("taskWorkflowPolicyShow", root, patchsetId, options);
  }

  taskWorkflowPolicyWaive(root, patchsetId, ruleName, reason, options) {
    return this.call("taskWorkflowPolicyWaive", root, patchsetId, ruleName, reason, options);
  }

  changeCreate(repoRoot, taskId, title, options) {
    return this.call("changeCreate", repoRoot, taskId, title, options);
  }

  changeShow(repoRoot, reference, options) {
    return this.call("changeShow", repoRoot, reference, options);
  }

  changeRevert(repoRoot, reference, options) {
    return this.call("changeRevert", repoRoot, reference, options);
  }

  changeReplay(repoRoot, reference, onto, options) {
    return this.call("changeReplay", repoRoot, reference, onto, options);
  }

  changeClose(repoRoot, reference, options) {
    return this.call("changeClose", repoRoot, reference, options);
  }

  changePublish(repoRoot, reference, remoteName) {
    return this.call("changePublish", repoRoot, reference, remoteName);
  }

  patchsetPublish(repoRoot, reference, summary, options) {
    return this.call("patchsetPublish", repoRoot, reference, summary, options);
  }

  patchsetList(repoRoot, reference, remoteName) {
    return this.call("patchsetList", repoRoot, reference, remoteName);
  }

  formatChangeCreate(payload) {
    return this.call("formatChangeCreate", payload);
  }

  formatChangeShow(payload) {
    return this.call("formatChangeShow", payload);
  }

  formatChangeRevert(payload) {
    return this.call("formatChangeRevert", payload);
  }

  formatChangeReplay(payload) {
    return this.call("formatChangeReplay", payload);
  }

  formatChangeClose(payload) {
    return this.call("formatChangeClose", payload);
  }

  formatChangePublish(payload) {
    return this.call("formatChangePublish", payload);
  }

  formatPatchsetPublish(payload) {
    return this.call("formatPatchsetPublish", payload);
  }

  formatPatchsetList(payload) {
    return this.call("formatPatchsetList", payload);
  }

  formatPatchsetShow(payload) {
    return this.call("formatPatchsetShow", payload);
  }

  formatPatchsetSelect(payload) {
    return this.call("formatPatchsetSelect", payload);
  }

  formatPatchsetCiStatus(payload) {
    return this.call("formatPatchsetCiStatus", payload);
  }

  formatPatchsetRerunCi(payload) {
    return this.call("formatPatchsetRerunCi", payload);
  }

  projectChangeList(payload, all) {
    return this.call("projectChangeList", payload, all);
  }

  formatChangeList(payload, options) {
    return this.call("formatChangeList", payload, options);
  }

  workspaceCommandLockPath(repoRoot) {
    return this.call("workspaceCommandLockPath", repoRoot);
  }

  taskWorkflowSnapshotChain(repoRoot, snapshotId) {
    return this.call("taskWorkflowSnapshotChain", repoRoot, snapshotId);
  }

  taskWorkflowSnapshotExists(repoRoot, snapshotId) {
    return this.call("taskWorkflowSnapshotExists", repoRoot, snapshotId);
  }

  taskWorkflowBlobReadBytes(repoRoot, blobId) {
    return this.call("taskWorkflowBlobReadBytes", repoRoot, blobId);
  }

  taskWorkflowBlobEnsureBytes(repoRoot, data, pathHint) {
    return this.call("taskWorkflowBlobEnsureBytes", repoRoot, data, pathHint);
  }

  taskWorkflowWorkspaceDelta(repoRoot, snapshotId) {
    return this.call("taskWorkflowWorkspaceDelta", repoRoot, snapshotId);
  }

  taskWorkflowWorkspaceRestore(repoRoot, options) {
    return this.call("taskWorkflowWorkspaceRestore", repoRoot, options);
  }

  taskWorkflowWorkspaceRestorePaths(repoRoot, paths, options) {
    return this.call("taskWorkflowWorkspaceRestorePaths", repoRoot, paths, options);
  }

  taskWorkflowSnapshotDiff(repoRoot, older, newer, options) {
    return this.call("taskWorkflowSnapshotDiff", repoRoot, older, newer, options);
  }

  taskWorkflowSnapshotRevert(repoRoot, snapshotId, options) {
    return this.call("taskWorkflowSnapshotRevert", repoRoot, snapshotId, options);
  }

  taskWorkflowSnapshotReplay(repoRoot, snapshotId, onto, options) {
    return this.call("taskWorkflowSnapshotReplay", repoRoot, snapshotId, onto, options);
  }

  taskWorkflowSnapshotCreateExplicit(repoRoot, repoName, lineName, options) {
    return this.call("taskWorkflowSnapshotCreateExplicit", repoRoot, repoName, lineName, options);
  }

  taskWorkflowSnapshotRestoreLines(repoRoot, snapshotId, path, options) {
    return this.call("taskWorkflowSnapshotRestoreLines", repoRoot, snapshotId, path, options);
  }

  snapshotDiff(repoRoot, olderSelector, newerSelector, options) {
    return this.call("snapshotDiff", repoRoot, olderSelector, newerSelector, options);
  }

  snapshotRevert(repoRoot, selector, options) {
    return this.call("snapshotRevert", repoRoot, selector, options);
  }

  snapshotReplay(repoRoot, selector, onto, options) {
    return this.call("snapshotReplay", repoRoot, selector, onto, options);
  }

  formatSnapshotDiff(payload) {
    return this.call("formatSnapshotDiff", payload);
  }

  formatSnapshotRevert(payload) {
    return this.call("formatSnapshotRevert", payload);
  }

  formatSnapshotReplay(payload) {
    return this.call("formatSnapshotReplay", payload);
  }

  formatSnapshotRestoreLines(payload) {
    return this.call("formatSnapshotRestoreLines", payload);
  }

  taskWorkflowWorktreeGet(root, options) {
    return this.call("taskWorkflowWorktreeGet", root, options);
  }

  taskWorkflowWorktreeStatus(root, options) {
    return this.call("taskWorkflowWorktreeStatus", root, options);
  }

  taskWorkflowWorktreeRestore(root, options) {
    return this.call("taskWorkflowWorktreeRestore", root, options);
  }

  taskWorkflowWorktreeList(root, options) {
    return this.call("taskWorkflowWorktreeList", root, options);
  }

  taskWorkflowWorktreeTouchUsage(root, options) {
    return this.call("taskWorkflowWorktreeTouchUsage", root, options);
  }

  taskWorkflowWorktreeDoctor(root, options) {
    return this.call("taskWorkflowWorktreeDoctor", root, options);
  }

  taskWorkflowWorktreeCleanupCandidates(root, options) {
    return this.call("taskWorkflowWorktreeCleanupCandidates", root, options);
  }

  taskWorkflowWorktreeCleanup(root, options) {
    return this.call("taskWorkflowWorktreeCleanup", root, options);
  }

  taskWorkflowWorktreePruneStale(root, options) {
    return this.call("taskWorkflowWorktreePruneStale", root, options);
  }

  taskWorkflowWorktreeRemove(root, names, options) {
    return this.call("taskWorkflowWorktreeRemove", root, names, options);
  }

  taskWorkflowWorktreeBindExisting(root, worktreeName, options) {
    return this.call("taskWorkflowWorktreeBindExisting", root, worktreeName, options);
  }

  taskWorkflowWorktreeSync(root, options) {
    return this.call("taskWorkflowWorktreeSync", root, options);
  }

  taskWorkflowWorktreeSyncAll(root, options) {
    return this.call("taskWorkflowWorktreeSyncAll", root, options);
  }

  taskWorkflowWorktreeRecreate(root, options) {
    return this.call("taskWorkflowWorktreeRecreate", root, options);
  }

  taskWorkflowWorktreeRestoreOwnedHead(root, options) {
    return this.call("taskWorkflowWorktreeRestoreOwnedHead", root, options);
  }

  taskWorkflowWorktreePreviewRebase(root, options) {
    return this.call("taskWorkflowWorktreePreviewRebase", root, options);
  }

  taskWorkflowWorktreeRebase(root, options) {
    return this.call("taskWorkflowWorktreeRebase", root, options);
  }

  taskWorkflowWorktreeContinueRebase(root, options) {
    return this.call("taskWorkflowWorktreeContinueRebase", root, options);
  }

  taskWorkflowWorktreeAbortRebase(root, options) {
    return this.call("taskWorkflowWorktreeAbortRebase", root, options);
  }

  taskWorkflowAuthWhoami(repoRoot, options) {
    return this.call("taskWorkflowAuthWhoami", repoRoot, options);
  }

  taskWorkflowAuthBindings(repoRoot, options) {
    return this.call("taskWorkflowAuthBindings", repoRoot, options);
  }

  taskWorkflowAuthGrant(repoRoot, actorIdentity, roles, options) {
    return this.call("taskWorkflowAuthGrant", repoRoot, actorIdentity, roles, options);
  }

  formatAuthWhoami(payload) {
    return this.call("formatAuthWhoami", payload);
  }

  formatAuthBindings(payload) {
    return this.call("formatAuthBindings", payload);
  }

  formatAuthGrant(payload) {
    return this.call("formatAuthGrant", payload);
  }

  stashSave(repoRoot, options) {
    return this.call("stashSave", repoRoot, options);
  }

  taskWorkflowStashList(repoRoot) {
    return this.call("taskWorkflowStashList", repoRoot);
  }

  taskWorkflowStashShow(repoRoot, stashId) {
    return this.call("taskWorkflowStashShow", repoRoot, stashId);
  }

  stashApply(repoRoot, stashId, force) {
    return this.call("stashApply", repoRoot, stashId, force);
  }

  stashPop(repoRoot, stashId, force) {
    return this.call("stashPop", repoRoot, stashId, force);
  }

  stashDrop(repoRoot, stashId) {
    return this.call("stashDrop", repoRoot, stashId);
  }

  taskWorkflowStashDrop(repoRoot, stashId) {
    return this.call("taskWorkflowStashDrop", repoRoot, stashId);
  }

  formatStashSave(payload) {
    return this.call("formatStashSave", payload);
  }

  formatStashList(payload) {
    return this.call("formatStashList", payload);
  }

  formatStashShow(payload) {
    return this.call("formatStashShow", payload);
  }

  formatStashApply(payload) {
    return this.call("formatStashApply", payload);
  }

  formatStashPop(payload) {
    return this.call("formatStashPop", payload);
  }

  formatStashDrop(payload) {
    return this.call("formatStashDrop", payload);
  }

  tagCreate(repoRoot, name, message, snapshotId) {
    return this.call("tagCreate", repoRoot, name, message, snapshotId);
  }

  tagList(repoRoot) {
    return this.call("tagList", repoRoot);
  }

  tagShow(repoRoot, name) {
    return this.call("tagShow", repoRoot, name);
  }

  tagDelete(repoRoot, name) {
    return this.call("tagDelete", repoRoot, name);
  }

  resolveSnapshotRef(repoRoot, value) {
    return this.call("resolveSnapshotRef", repoRoot, value);
  }

  formatTagCreate(payload) {
    return this.call("formatTagCreate", payload);
  }

  formatTagList(payload) {
    return this.call("formatTagList", payload);
  }

  formatTagShow(payload) {
    return this.call("formatTagShow", payload);
  }

  formatTagDelete(payload) {
    return this.call("formatTagDelete", payload);
  }

  reviewShow(repoRoot, reference, remoteName) {
    return this.call("reviewShow", repoRoot, reference, remoteName);
  }

  reviewTeamRequest(repoRoot, reference, reviewerGroups, options) {
    return this.call("reviewTeamRequest", repoRoot, reference, reviewerGroups, options);
  }

  reviewTeamApprove(repoRoot, reference, options) {
    return this.call("reviewTeamApprove", repoRoot, reference, options);
  }

  reviewTeamRequestChanges(repoRoot, reference, options) {
    return this.call("reviewTeamRequestChanges", repoRoot, reference, options);
  }

  reviewTeamComment(repoRoot, reference, options) {
    return this.call("reviewTeamComment", repoRoot, reference, options);
  }

  reviewTeamDefer(repoRoot, reference, options) {
    return this.call("reviewTeamDefer", repoRoot, reference, options);
  }

  reviewTaskApprove(repoRoot, reference, patchsetSelector, message, remoteName) {
    return this.call("reviewTaskApprove", repoRoot, reference, patchsetSelector, message, remoteName);
  }

  reviewTaskRequestChanges(repoRoot, reference, options) {
    return this.call("reviewTaskRequestChanges", repoRoot, reference, options);
  }

  reviewTaskComment(repoRoot, reference, options) {
    return this.call("reviewTaskComment", repoRoot, reference, options);
  }

  reviewTaskDefer(repoRoot, reference, options) {
    return this.call("reviewTaskDefer", repoRoot, reference, options);
  }

  reviewCodeSubmit(repoRoot, reference, patchsetId, message, options) {
    return this.call("reviewCodeSubmit", repoRoot, reference, patchsetId, message, options);
  }

  formatReviewTaskApprove(operation) {
    return this.call("formatReviewTaskApprove", operation);
  }

  formatReviewTaskRequestChanges(operation) {
    return this.call("formatReviewTaskRequestChanges", operation);
  }

  formatReviewTaskComment(operation) {
    return this.call("formatReviewTaskComment", operation);
  }

  formatReviewTaskDefer(operation) {
    return this.call("formatReviewTaskDefer", operation);
  }

  formatReviewTeamRequest(payload) {
    return this.call("formatReviewTeamRequest", payload);
  }

  formatReviewTeamApprove(payload) {
    return this.call("formatReviewTeamApprove", payload);
  }

  formatReviewTeamRequestChanges(payload) {
    return this.call("formatReviewTeamRequestChanges", payload);
  }

  formatReviewTeamComment(payload) {
    return this.call("formatReviewTeamComment", payload);
  }

  formatReviewTeamDefer(payload) {
    return this.call("formatReviewTeamDefer", payload);
  }

  formatReviewShow(payload) {
    return this.call("formatReviewShow", payload);
  }

  formatReviewCodeSubmit(payload) {
    return this.call("formatReviewCodeSubmit", payload);
  }

  formatReviewCodeTemplate(payload) {
    return this.call("formatReviewCodeTemplate", payload);
  }

  projectReviewTaskOperation(operation) {
    return this.call("projectReviewTaskOperation", operation);
  }

  projectReviewShow(payload) {
    return this.call("projectReviewShow", payload);
  }

  validateReviewPatchsetSelector(value) {
    return this.call("validateReviewPatchsetSelector", value);
  }

  validatePublicPatchsetArgument(value) {
    return this.call("validatePublicPatchsetArgument", value);
  }

  formatAttestPut(payload) {
    return this.call("formatAttestPut", payload);
  }

  formatAttestShow(payload) {
    return this.call("formatAttestShow", payload);
  }

  formatPolicyEval(payload) {
    return this.call("formatPolicyEval", payload);
  }

  formatPolicyShow(payload) {
    return this.call("formatPolicyShow", payload);
  }

  formatPolicyWaive(payload) {
    return this.call("formatPolicyWaive", payload);
  }

  projectPolicyShow(payload) {
    return this.call("projectPolicyShow", payload);
  }

  workspaceDiff(repoRoot, paths) {
    return this.call("workspaceDiff", repoRoot, paths);
  }

  renderWorkspaceDiffNames(payload) {
    return this.call("renderWorkspaceDiffNames", payload);
  }

  renderWorkspaceDiffStat(payload) {
    return this.call("renderWorkspaceDiffStat", payload);
  }

  renderWorkspaceDiffText(payload) {
    return this.call("renderWorkspaceDiffText", payload);
  }

  taskWorkflowBlame(repoRoot, path, options) {
    return this.call("taskWorkflowBlame", repoRoot, path, options);
  }

  formatBlame(payload) {
    return this.call("formatBlame", payload);
  }

  pullRepository(root, options) {
    return this.call("pullRepository", root, options);
  }

  pushRepository(root, options) {
    return this.call("pushRepository", root, options);
  }

  taskWorkflowPull(root, options) {
    return this.call("taskWorkflowPull", root, options);
  }

  taskWorkflowPush(root, options) {
    return this.call("taskWorkflowPush", root, options);
  }

  taskWorkflowUploadSnapshotChain(root, snapshotId, options) {
    return this.call("taskWorkflowUploadSnapshotChain", root, snapshotId, options);
  }

  formatPull(payload) {
    return this.call("formatPull", payload);
  }

  formatPush(payload) {
    return this.call("formatPush", payload);
  }

  projectRemoteSync(payload) {
    return this.call("projectRemoteSync", payload);
  }

  worktreeShow(root, name) {
    return this.call("worktreeShow", root, name);
  }

  worktreePath(root, name) {
    return this.call("worktreePath", root, name);
  }

  worktreeRestore(root, options) {
    return this.call("worktreeRestore", root, options);
  }

  worktreeCleanup(root, options) {
    return this.call("worktreeCleanup", root, options);
  }

  worktreePruneStale(root, options) {
    return this.call("worktreePruneStale", root, options);
  }

  worktreeSync(root, options) {
    return this.call("worktreeSync", root, options);
  }

  worktreeRecreate(root, options) {
    return this.call("worktreeRecreate", root, options);
  }

  worktreeRecoverTask(root, taskId, options) {
    return this.call("worktreeRecoverTask", root, taskId, options);
  }

  worktreeRestoreOwnedHead(root, options) {
    return this.call("worktreeRestoreOwnedHead", root, options);
  }

  worktreeRebase(root, options) {
    return this.call("worktreeRebase", root, options);
  }

  worktreeRemove(root, names, options) {
    return this.call("worktreeRemove", root, names, options);
  }

  formatWorktreeStatus(payload, verbose) {
    return this.call("formatWorktreeStatus", payload, verbose);
  }

  formatWorktreePath(payload, shellOutput) {
    return this.call("formatWorktreePath", payload, shellOutput);
  }

  formatWorktreeShow(payload) {
    return this.call("formatWorktreeShow", payload);
  }

  formatWorktreeList(payload) {
    return this.call("formatWorktreeList", payload);
  }

  formatWorktreeDoctor(payload) {
    return this.call("formatWorktreeDoctor", payload);
  }

  formatWorktreeCleanupCandidates(payload) {
    return this.call("formatWorktreeCleanupCandidates", payload);
  }

  formatWorktreeCleanup(payload) {
    return this.call("formatWorktreeCleanup", payload);
  }

  formatWorktreePruneStale(payload) {
    return this.call("formatWorktreePruneStale", payload);
  }

  formatWorktreeSync(payload) {
    return this.call("formatWorktreeSync", payload);
  }

  formatWorktreeRestore(payload) {
    return this.call("formatWorktreeRestore", payload);
  }

  formatWorktreeRecreate(payload) {
    return this.call("formatWorktreeRecreate", payload);
  }

  formatWorktreeRecoverTask(payload) {
    return this.call("formatWorktreeRecoverTask", payload);
  }

  formatWorktreeRestoreOwnedHead(payload) {
    return this.call("formatWorktreeRestoreOwnedHead", payload);
  }

  formatWorktreeRebase(payload) {
    return this.call("formatWorktreeRebase", payload);
  }

  formatWorktreeRemove(payload, allStale, dryRun, multiple) {
    return this.call("formatWorktreeRemove", payload, allStale, dryRun, multiple);
  }

  projectWorktreePath(payload) {
    return this.call("projectWorktreePath", payload);
  }

  worktreeSyncExitCode(payload, allWorktrees) {
    return this.call("worktreeSyncExitCode", payload, allWorktrees);
  }

  gitMigrate(root, stagingRoot, apply) {
    return this.call("gitMigrate", root, stagingRoot, apply);
  }

  gitRecover(root, apply) {
    return this.call("gitRecover", root, apply);
  }

  gitImport(root, source, options) {
    return this.call("gitImport", root, source, options);
  }

  gitExport(root, target, options) {
    return this.call("gitExport", root, target, options);
  }

  gitRestore(root, generation, gitObject, target) {
    return this.call("gitRestore", root, generation, gitObject, target);
  }

  gitMirror(root, endpoint, direction, dryRun) {
    return this.call("gitMirror", root, endpoint, direction, dryRun);
  }

  formatGitMigrate(payload) {
    return this.call("formatGitMigrate", payload);
  }

  formatGitRecover(payload) {
    return this.call("formatGitRecover", payload);
  }

  formatGitImport(payload) {
    return this.call("formatGitImport", payload);
  }

  formatGitExport(payload) {
    return this.call("formatGitExport", payload);
  }

  formatGitRestore(payload) {
    return this.call("formatGitRestore", payload);
  }

  formatGitMirror(payload) {
    return this.call("formatGitMirror", payload);
  }

  gcStats(root) {
    return this.call("gcStats", root);
  }

  gcValidate(root) {
    return this.call("gcValidate", root);
  }

  gcPrune(root, apply) {
    return this.call("gcPrune", root, apply);
  }

  gcValidateExitCode(payload) {
    return this.call("gcValidateExitCode", payload);
  }

  formatGcStats(payload) {
    return this.call("formatGcStats", payload);
  }

  formatGcValidate(payload) {
    return this.call("formatGcValidate", payload);
  }

  formatGcPrune(payload) {
    return this.call("formatGcPrune", payload);
  }

  policyProfileNames() {
    return this.call("policyProfileNames");
  }

  authorModeValues() {
    return this.call("authorModeValues");
  }

  policyContentClassValues() {
    return this.call("policyContentClassValues");
  }

  policyAuthorClassValues() {
    return this.call("policyAuthorClassValues");
  }

  normalizeAuthorMode(value) {
    return this.call("normalizeAuthorMode", value);
  }

  missingCodeReviewSummarySections(value) {
    return this.call("missingCodeReviewSummarySections", value);
  }

  isStructuredCodeReviewSummary(value) {
    return this.call("isStructuredCodeReviewSummary", value);
  }

  renderCodeReviewSummaryTemplate(style) {
    return this.call("renderCodeReviewSummaryTemplate", style);
  }

  codeReviewSummaryRequirementText(value) {
    return this.call("codeReviewSummaryRequirementText", value);
  }

  derivePolicyContentClass(changedPaths) {
    return this.call("derivePolicyContentClass", changedPaths);
  }

  derivePolicyAuthorClass(authorMode) {
    return this.call("derivePolicyAuthorClass", authorMode);
  }

  buildMinimumProvenance(authorMode, modelName) {
    return this.call("buildMinimumProvenance", authorMode, modelName);
  }

  policyProfile(name) {
    return this.call("policyProfile", name);
  }

  normalizePolicy(policy, fallbackProfile) {
    return this.call("normalizePolicy", policy, fallbackProfile);
  }

  resolveEffectivePolicy(policy, options) {
    return this.call("resolveEffectivePolicy", policy, options);
  }

  policyToYaml(policy, fallbackProfile) {
    return this.call("policyToYaml", policy, fallbackProfile);
  }

  parsePolicyYaml(text, fallbackProfile) {
    return this.call("parsePolicyYaml", text, fallbackProfile);
  }

  runtimeDataEnvValue() {
    return this.call("runtimeDataEnvValue");
  }

  runtimeDataEnvName() {
    return this.call("runtimeDataEnvName");
  }

  resolveRuntimeDataRootWithSource(path) {
    return this.call("resolveRuntimeDataRootWithSource", path);
  }

  resolveRuntimeDataRoot(path) {
    return this.call("resolveRuntimeDataRoot", path);
  }

  resolveServerRuntimeRootWithSource(path) {
    return this.call("resolveServerRuntimeRootWithSource", path);
  }

  resolveServerRuntimeRoot(path) {
    return this.call("resolveServerRuntimeRoot", path);
  }

  encodeRefName(name) {
    return this.call("encodeRefName", name);
  }

  decodeRefName(name) {
    return this.call("decodeRefName", name);
  }

  normalizeTaskStatus(value) {
    return this.call("normalizeTaskStatus", value);
  }

  normalizeOptionalText(value) {
    return this.call("normalizeOptionalText", value);
  }

  utcNow() {
    return this.call("utcNow");
  }

  generateNamespacedWorkflowId(family, namespacePrefix) {
    return this.call("generateNamespacedWorkflowId", family, namespacePrefix);
  }

  normalizeIdNamespacePrefix(value, defaultPrefix) {
    return this.call("normalizeIdNamespacePrefix", value, defaultPrefix);
  }

  workflowIdTokens(family, options) {
    return this.call("workflowIdTokens", family, options);
  }

  workflowOriginNamespacePrefix(originPrefix, namespacePrefix) {
    return this.call("workflowOriginNamespacePrefix", originPrefix, namespacePrefix);
  }

  workflowIdNamespacePrefixCandidates(options) {
    return this.call("workflowIdNamespacePrefixCandidates", options);
  }

  workflowIdNamespacePrefixForValue(family, value, options) {
    return this.call("workflowIdNamespacePrefixForValue", family, value, options);
  }

  workflowIdMatchesAnyNamespacePrefix(family, value, options) {
    return this.call("workflowIdMatchesAnyNamespacePrefix", family, value, options);
  }

  derivePatchsetId(changeId, patchsetNumber, namespacePrefix) {
    return this.call("derivePatchsetId", changeId, patchsetNumber, namespacePrefix);
  }

  generateWorkflowId(prefix) {
    return this.call("generateWorkflowId", prefix);
  }

  taskStatusDetails(value) {
    return this.call("taskStatusDetails", value);
  }

  normalizePublicationState(value) {
    return this.call("normalizePublicationState", value);
  }

  publicationStateHasUnpublishedHead(value) {
    return this.call("publicationStateHasUnpublishedHead", value);
  }

  workflowIdToken(family, namespacePrefix) {
    return this.call("workflowIdToken", family, namespacePrefix);
  }

  workflowIdMatches(family, value, options) {
    return this.call("workflowIdMatches", family, value, options);
  }

  generateNamespacedSequenceId(family, number, options) {
    return this.call("generateNamespacedSequenceId", family, number, options);
  }

  workflowSuccessEnvelope(kind, value) {
    return this.call("workflowSuccessEnvelope", kind, value);
  }

  workflowErrorEnvelope(kind, code, message, detail) {
    return this.call("workflowErrorEnvelope", kind, code, message, detail);
  }

  parsePlanMarkdown(markdown) {
    return this.call("parsePlanMarkdown", markdown);
  }

  extractPlanRefs(parsedPlan) {
    return this.call("extractPlanRefs", parsedPlan);
  }

  computeSyncPruneDecisions(scope, trackedArtifacts, syncedArtifacts) {
    return this.call("computeSyncPruneDecisions", scope, trackedArtifacts, syncedArtifacts);
  }

  diffSnapshotManifests(oldFiles, newFiles, options) {
    return this.call("diffSnapshotManifests", oldFiles, newFiles, options);
  }

  snapshotDiffFromManifests(oldFiles, newFiles, options) {
    return this.call("snapshotDiffFromManifests", oldFiles, newFiles, options);
  }

  currentCoreSourceFingerprint(coreRepoRoot) {
    return this.call("currentCoreSourceFingerprint", coreRepoRoot);
  }

  currentServerSourceFingerprint(serverCoreRepoRoot) {
    return this.call("currentServerSourceFingerprint", serverCoreRepoRoot);
  }

  currentSourceNativeCacheContract(request) {
    return this.call("currentSourceNativeCacheContract", request);
  }

  currentSourceCacheMarkBuilding(request, sourceMtimeNs, extraJson) {
    return this.call("currentSourceCacheMarkBuilding", request, sourceMtimeNs, extraJson);
  }

  currentSourceCacheActivate(request, sourceMtimeNs, options) {
    return this.call("currentSourceCacheActivate", request, sourceMtimeNs, options);
  }

  currentSourceCacheReleaseLease(leasePath, namespaceRoot, removeUnleasedReady) {
    return this.call("currentSourceCacheReleaseLease", leasePath, namespaceRoot, removeUnleasedReady);
  }

  currentSourceCachePrune(namespaceRoot, options) {
    return this.call("currentSourceCachePrune", namespaceRoot, options);
  }

  currentSourceCacheSeedCanonical(request, repoRoot, canonicalRepoRoot, coreSourceMtimeNs) {
    return this.call("currentSourceCacheSeedCanonical", request, repoRoot, canonicalRepoRoot, coreSourceMtimeNs);
  }

  currentSourceCacheExtensionFresh(request) {
    return this.call("currentSourceCacheExtensionFresh", request);
  }

  currentSourceCacheBinaryFresh(request) {
    return this.call("currentSourceCacheBinaryFresh", request);
  }

  currentSourceCacheCoreFingerprint(repoRoot) {
    return this.call("currentSourceCacheCoreFingerprint", repoRoot);
  }

  currentSourceCacheServerFingerprint(repoRoot) {
    return this.call("currentSourceCacheServerFingerprint", repoRoot);
  }

  currentSourceCacheCoreMtime(repoRoot) {
    return this.call("currentSourceCacheCoreMtime", repoRoot);
  }

  currentSourceCacheServerMtime(repoRoot) {
    return this.call("currentSourceCacheServerMtime", repoRoot);
  }

  formatCurrentSourceCacheContract(payload) {
    return this.call("formatCurrentSourceCacheContract", payload);
  }

  formatCurrentSourceCacheMarkBuilding(payload) {
    return this.call("formatCurrentSourceCacheMarkBuilding", payload);
  }

  formatCurrentSourceCacheActivate(payload) {
    return this.call("formatCurrentSourceCacheActivate", payload);
  }

  formatCurrentSourceCacheReleaseLease(payload) {
    return this.call("formatCurrentSourceCacheReleaseLease", payload);
  }

  formatCurrentSourceCachePrune(payload) {
    return this.call("formatCurrentSourceCachePrune", payload);
  }

  formatCurrentSourceCacheSeedCanonical(payload) {
    return this.call("formatCurrentSourceCacheSeedCanonical", payload);
  }

  formatCurrentSourceCacheExtensionFresh(payload) {
    return this.call("formatCurrentSourceCacheExtensionFresh", payload);
  }

  formatCurrentSourceCacheBinaryFresh(payload) {
    return this.call("formatCurrentSourceCacheBinaryFresh", payload);
  }

  formatCurrentSourceCacheCoreFingerprint(payload) {
    return this.call("formatCurrentSourceCacheCoreFingerprint", payload);
  }

  formatCurrentSourceCacheServerFingerprint(payload) {
    return this.call("formatCurrentSourceCacheServerFingerprint", payload);
  }

  formatCurrentSourceCacheCoreMtime(payload) {
    return this.call("formatCurrentSourceCacheCoreMtime", payload);
  }

  formatCurrentSourceCacheServerMtime(payload) {
    return this.call("formatCurrentSourceCacheServerMtime", payload);
  }

  extractPlanItems(markdown) {
    return this.call("extractPlanItems", markdown);
  }

  listPlanSectionRefs(markdown) {
    return this.call("listPlanSectionRefs", markdown);
  }

  extractPlanSection(markdown, planRef) {
    return this.call("extractPlanSection", markdown, planRef);
  }

  findPlanItem(markdown, planItemRef) {
    return this.call("findPlanItem", markdown, planItemRef);
  }

  normalizePlanItems(items) {
    return this.call("normalizePlanItems", items);
  }

  findPlanItemInItems(items, planItemRef) {
    return this.call("findPlanItemInItems", items, planItemRef);
  }

  normalizeTaskWorkflowHttpCompatibility(payloadJson) {
    return this.call("normalizeTaskWorkflowHttpCompatibility", payloadJson);
  }

  normalizeTaskWorkflowHttpReadiness(payloadJson) {
    return this.call("normalizeTaskWorkflowHttpReadiness", payloadJson);
  }

  planItemsPayload(plan, revision) {
    return this.call("planItemsPayload", plan, revision);
  }

  localPlanPublishShadow(plan) {
    return this.call("localPlanPublishShadow", plan);
  }

  planDispatchSummary(plan, options) {
    return this.call("planDispatchSummary", plan, options);
  }

  computeTaskableItems(plan, options) {
    return this.call("computeTaskableItems", plan, options);
  }

  validateDispatchLegality(plan, planItemRef, options) {
    return this.call("validateDispatchLegality", plan, planItemRef, options);
  }

  planTaskLinkIndexes(tasks) {
    return this.call("planTaskLinkIndexes", tasks);
  }

  planCandidatesPayload(summaries, options) {
    return this.call("planCandidatesPayload", summaries, options);
  }

  artifactBlobId(markdown) {
    return this.call("artifactBlobId", markdown);
  }

  artifactCandidatesOpen(candidates) {
    return this.call("artifactCandidatesOpen", candidates);
  }

  indexPlansByArtifactPath(plans) {
    return this.call("indexPlansByArtifactPath", plans);
  }

  indexPlansByArtifactIdentity(plans) {
    return this.call("indexPlansByArtifactIdentity", plans);
  }

  planArtifactIdentity(artifactPath, artifactSelector) {
    return this.call("planArtifactIdentity", artifactPath, artifactSelector);
  }

  planArtifactIdentityLabel(artifactPath, artifactSelector) {
    return this.call("planArtifactIdentityLabel", artifactPath, artifactSelector);
  }

  openGenericPlansMatchingBlobId(plans, blobId) {
    return this.call("openGenericPlansMatchingBlobId", plans, blobId);
  }

  openPlansMatchingSelector(plans, selector) {
    return this.call("openPlansMatchingSelector", plans, selector);
  }

  localPlanFullyPublished(plan) {
    return this.call("localPlanFullyPublished", plan);
  }

  planHeadsEquivalent(left, right) {
    return this.call("planHeadsEquivalent", left, right);
  }

  planMatchesSyncArtifact(plan, artifact, requireTitleMatch) {
    return this.call("planMatchesSyncArtifact", plan, artifact, requireTitleMatch);
  }

  planFilesystemNormalizeMarkdownArtifactPath(pathValue) {
    return this.call("planFilesystemNormalizeMarkdownArtifactPath", pathValue);
  }

  planFilesystemIsMarkdownArtifactPath(pathValue) {
    return this.call("planFilesystemIsMarkdownArtifactPath", pathValue);
  }

  planFilesystemIsLineageOnlyMarkdownArtifactPath(pathValue) {
    return this.call("planFilesystemIsLineageOnlyMarkdownArtifactPath", pathValue);
  }

  planFilesystemReadUtf8TextFile(pathValue) {
    return this.call("planFilesystemReadUtf8TextFile", pathValue);
  }

  planFilesystemReadJsonFile(pathValue) {
    return this.call("planFilesystemReadJsonFile", pathValue);
  }

  planFilesystemReadBinaryFile(pathValue) {
    return this.call("planFilesystemReadBinaryFile", pathValue);
  }

  planFilesystemPathIsProjectedOutForWorkspace(repoRoot, relPath, isWorktree) {
    return this.call("planFilesystemPathIsProjectedOutForWorkspace", repoRoot, relPath, isWorktree);
  }

  planFilesystemWorkspacePathIsIgnored(repoRoot, pathValue, ignoreRulesText) {
    return this.call("planFilesystemWorkspacePathIsIgnored", repoRoot, pathValue, ignoreRulesText);
  }

  planFilesystemListVisibleWorkspacePaths(repoRoot, options) {
    return this.call("planFilesystemListVisibleWorkspacePaths", repoRoot, options);
  }

  planFilesystemListVisibleMarkdownArtifactPaths(repoRoot, options) {
    return this.call("planFilesystemListVisibleMarkdownArtifactPaths", repoRoot, options);
  }

  planFilesystemZipArchiveHasMember(pathValue, entryName) {
    return this.call("planFilesystemZipArchiveHasMember", pathValue, entryName);
  }

  planFilesystemReadZipArchiveMember(pathValue, entryName) {
    return this.call("planFilesystemReadZipArchiveMember", pathValue, entryName);
  }

  planFilesystemResolveRepoArtifactPath(repoRoot, pathValue, allowMissing) {
    return this.call("planFilesystemResolveRepoArtifactPath", repoRoot, pathValue, allowMissing);
  }

  repoStateReadRepoConfigJson(pathValue) {
    return this.call("repoStateReadRepoConfigJson", pathValue);
  }

  repoStateWriteRepoConfigJson(pathValue, payload) {
    return this.call("repoStateWriteRepoConfigJson", pathValue, payload);
  }

  repoStateReadWorktreeConfigJson(pathValue) {
    return this.call("repoStateReadWorktreeConfigJson", pathValue);
  }

  repoStateWriteWorktreeConfigJson(pathValue, payload) {
    return this.call("repoStateWriteWorktreeConfigJson", pathValue, payload);
  }

  repoStateReadWorktreeMetadataJson(pathValue) {
    return this.call("repoStateReadWorktreeMetadataJson", pathValue);
  }

  repoStateWriteWorktreeMetadataJson(pathValue, payload) {
    return this.call("repoStateWriteWorktreeMetadataJson", pathValue, payload);
  }

  planPackSubstrateBuildPackMembers(blobItems, options) {
    return this.call("planPackSubstrateBuildPackMembers", blobItems, options);
  }

  planPackSubstrateBuildTreePackMembers(treeRows, treeEntryRows) {
    return this.call("planPackSubstrateBuildTreePackMembers", treeRows, treeEntryRows);
  }

  planPackSubstrateBuildGitBinaryDelta(baseData, targetData) {
    return this.call("planPackSubstrateBuildGitBinaryDelta", baseData, targetData);
  }

  planPackSubstrateBuildGitBinaryDeltaMember(entryName, blobId, baseBlobId, baseData, targetData, chainDepth) {
    return this.call("planPackSubstrateBuildGitBinaryDeltaMember", entryName, blobId, baseBlobId, baseData, targetData, chainDepth);
  }

  planPackSubstrateApplyGitBinaryDelta(baseData, deltaData) {
    return this.call("planPackSubstrateApplyGitBinaryDelta", baseData, deltaData);
  }

  planPackSubstrateApplyPackDelta(baseData, deltaData, algorithm) {
    return this.call("planPackSubstrateApplyPackDelta", baseData, deltaData, algorithm);
  }

  planPackSubstrateWritePackArchive(packPath, packId, createdAt, members) {
    return this.call("planPackSubstrateWritePackArchive", packPath, packId, createdAt, members);
  }

  planPackSubstrateWriteTreePackArchive(packPath, packId, createdAt, members) {
    return this.call("planPackSubstrateWriteTreePackArchive", packPath, packId, createdAt, members);
  }

  planPackSubstrateReadPackIndex(packPath) {
    return this.call("planPackSubstrateReadPackIndex", packPath);
  }

  planPackSubstrateReadTreePackIndex(packPath) {
    return this.call("planPackSubstrateReadTreePackIndex", packPath);
  }

  planPackSubstratePackHasEntry(packPath, entryName) {
    return this.call("planPackSubstratePackHasEntry", packPath, entryName);
  }

  planPackSubstrateReadTreePackTree(packPath, treeId) {
    return this.call("planPackSubstrateReadTreePackTree", packPath, treeId);
  }

  planPackSubstrateSummarizePackArchives(root, packRows) {
    return this.call("planPackSubstrateSummarizePackArchives", root, packRows);
  }

  planPackSubstrateSummarizeTreePackArchives(root, packRows) {
    return this.call("planPackSubstrateSummarizeTreePackArchives", root, packRows);
  }

  planPackSubstrateTreePackManifestPath(packPath, entryName) {
    return this.call("planPackSubstrateTreePackManifestPath", packPath, entryName);
  }

  planPackSubstrateReadTreePackTreeByOrdinal(packPath, entryOrdinal) {
    return this.call("planPackSubstrateReadTreePackTreeByOrdinal", packPath, entryOrdinal);
  }

  planPackSubstrateBuildStorageValidationSummary(options) {
    return this.call("planPackSubstrateBuildStorageValidationSummary", options);
  }

  planPackSubstrateBuildTreeRecords(fileEntries) {
    return this.call("planPackSubstrateBuildTreeRecords", fileEntries);
  }

  planPackSubstrateBuildSnapshotId(options) {
    return this.call("planPackSubstrateBuildSnapshotId", options);
  }

  joinPath(baseUrl, path) {
    return this.call("joinPath", baseUrl, path);
  }

  buildQueryString(params) {
    return this.call("buildQueryString", params);
  }

  normalizeRemoteText(value) {
    return this.call("normalizeRemoteText", value);
  }

  buildListPlansRequest(baseUrl, repoName, artifactPath) {
    return this.call("buildListPlansRequest", baseUrl, repoName, artifactPath);
  }

  buildGetPlanRequest(baseUrl, planId) {
    return this.call("buildGetPlanRequest", baseUrl, planId);
  }

  buildListPlanRevisionsRequest(baseUrl, planId) {
    return this.call("buildListPlanRevisionsRequest", baseUrl, planId);
  }

  buildGetPlanRevisionRequest(baseUrl, planId, planRevisionId) {
    return this.call("buildGetPlanRevisionRequest", baseUrl, planId, planRevisionId);
  }

  buildCreatePlanRequest(baseUrl, input) {
    return this.call("buildCreatePlanRequest", baseUrl, input);
  }

  buildRevisePlanRequest(baseUrl, input) {
    return this.call("buildRevisePlanRequest", baseUrl, input);
  }

  buildUpdatePlanStatusRequest(baseUrl, planId, status) {
    return this.call("buildUpdatePlanStatusRequest", baseUrl, planId, status);
  }

  buildPutPlanRevisionArtifactsRequest(baseUrl, planId, planRevisionId, artifacts) {
    return this.call("buildPutPlanRevisionArtifactsRequest", baseUrl, planId, planRevisionId, artifacts);
  }

  listPlans(baseUrl, repoName, artifactPath, options) {
    return this.call("listPlans", baseUrl, repoName, artifactPath, options);
  }

  getPlan(baseUrl, planId, options) {
    return this.call("getPlan", baseUrl, planId, options);
  }

  listPlanRevisions(baseUrl, planId, options) {
    return this.call("listPlanRevisions", baseUrl, planId, options);
  }

  getPlanRevision(baseUrl, planId, planRevisionId, options) {
    return this.call("getPlanRevision", baseUrl, planId, planRevisionId, options);
  }

  resolveTaskPlanLinkage(baseUrl, repoName, linkage, options) {
    return this.call("resolveTaskPlanLinkage", baseUrl, repoName, linkage, options);
  }

  listPlanIdsMatchingContains(baseUrl, repoName, containsTerms, options) {
    return this.call("listPlanIdsMatchingContains", baseUrl, repoName, containsTerms, options);
  }

  createPlan(baseUrl, input, options) {
    return this.call("createPlan", baseUrl, input, options);
  }

  revisePlan(baseUrl, input, options) {
    return this.call("revisePlan", baseUrl, input, options);
  }

  updatePlanStatus(baseUrl, planId, status, options) {
    return this.call("updatePlanStatus", baseUrl, planId, status, options);
  }

  putPlanRevisionArtifacts(baseUrl, planId, planRevisionId, artifacts, options) {
    return this.call("putPlanRevisionArtifacts", baseUrl, planId, planRevisionId, artifacts, options);
  }

  createPlanningSession(baseUrl, planId, input, options) {
    return this.call("createPlanningSession", baseUrl, planId, input, options);
  }

  listPlanningSessions(baseUrl, planId, status, options) {
    return this.call("listPlanningSessions", baseUrl, planId, status, options);
  }

  getPlanningSession(baseUrl, planningSessionId, options) {
    return this.call("getPlanningSession", baseUrl, planningSessionId, options);
  }

  appendPlanningSessionEvent(baseUrl, planningSessionId, eventType, payload, options) {
    return this.call("appendPlanningSessionEvent", baseUrl, planningSessionId, eventType, payload, options);
  }

  listPlanningSessionEvents(baseUrl, planningSessionId, input, options) {
    return this.call("listPlanningSessionEvents", baseUrl, planningSessionId, input, options);
  }

  joinPlanningSession(baseUrl, planningSessionId, input, options) {
    return this.call("joinPlanningSession", baseUrl, planningSessionId, input, options);
  }

  promotePlanningSession(baseUrl, planningSessionId, input, options) {
    return this.call("promotePlanningSession", baseUrl, planningSessionId, input, options);
  }

  closePlanningSession(baseUrl, planningSessionId, status, options) {
    return this.call("closePlanningSession", baseUrl, planningSessionId, status, options);
  }

  extractCodexUsageJsonl(usageJsonlPath, usageScope) {
    return this.call("extractCodexUsageJsonl", usageJsonlPath, usageScope);
  }

  extractCodexUsageBundleJsonl(usageJsonlPaths, options) {
    return this.call("extractCodexUsageBundleJsonl", usageJsonlPaths, options);
  }

  projectRepoRetireRuntimeBlockers(payload) {
    return this.call("projectRepoRetireRuntimeBlockers", payload);
  }

  languageBindingInfo() {
    return this.call("languageBindingInfo");
  }

  transportEnvelopeIrVersion() {
    return this.call("transportEnvelopeIrVersion");
  }

  transportEnvelopeSchema() {
    return this.call("transportEnvelopeSchema");
  }

  runtimeBindingStateIrVersion() {
    return this.call("runtimeBindingStateIrVersion");
  }

  runtimeBindingStateSchema() {
    return this.call("runtimeBindingStateSchema");
  }

  runtimeBindingStateDefaultPayload() {
    return this.call("runtimeBindingStateDefaultPayload");
  }

  runtimeBindingStateNormalizeDocument(payload) {
    return this.call("runtimeBindingStateNormalizeDocument", payload);
  }

  workerManifestIrVersion() {
    return this.call("workerManifestIrVersion");
  }

  workerManifestSchema() {
    return this.call("workerManifestSchema");
  }

  workerManifestDefaultConfig() {
    return this.call("workerManifestDefaultConfig");
  }

  workerManifestNormalizeDocument(payload, path) {
    return this.call("workerManifestNormalizeDocument", payload, path);
  }

  workerManifestSelectTelegramWorker(config, requestedName) {
    return this.call("workerManifestSelectTelegramWorker", config, requestedName);
  }

  transportEnvelopeBuildBindingMetadata(input) {
    return this.call("transportEnvelopeBuildBindingMetadata", input);
  }

  transportEnvelopeBuildEventEnvelope(input) {
    return this.call("transportEnvelopeBuildEventEnvelope", input);
  }

  transportEnvelopeBuildReplyEnvelope(input) {
    return this.call("transportEnvelopeBuildReplyEnvelope", input);
  }

  transportEnvelopeCompactEventEnvelope(envelope) {
    return this.call("transportEnvelopeCompactEventEnvelope", envelope);
  }

  transportEnvelopeCompactReplyEnvelope(envelope) {
    return this.call("transportEnvelopeCompactReplyEnvelope", envelope);
  }

  taskWorkflowTimestampFacts(now) {
    return this.call("taskWorkflowTimestampFacts", now);
  }

  taskWorkflowSequenceIdentityFacts(family, number, options) {
    return this.call("taskWorkflowSequenceIdentityFacts", family, number, options);
  }

  taskWorkflowWorkflowIdFacts(family, options) {
    return this.call("taskWorkflowWorkflowIdFacts", family, options);
  }

  taskWorkflowRuntimeSelectionFacts(overrides) {
    return this.call("taskWorkflowRuntimeSelectionFacts", overrides);
  }

  taskWorkflowReadUtf8TextFile(pathValue) {
    return this.call("taskWorkflowReadUtf8TextFile", pathValue);
  }

  taskWorkflowReadJsonFile(pathValue) {
    return this.call("taskWorkflowReadJsonFile", pathValue);
  }

  taskWorkflowReadBinaryFile(pathValue) {
    return this.call("taskWorkflowReadBinaryFile", pathValue);
  }

  taskWorkflowResolveRepoArtifactPath(repoRoot, pathValue, allowMissing) {
    return this.call("taskWorkflowResolveRepoArtifactPath", repoRoot, pathValue, allowMissing);
  }

  taskRemoteChangeLineagePayload(baseLine, lineRow) {
    return this.call("taskRemoteChangeLineagePayload", baseLine, lineRow);
  }

  taskWorkflowResolveTaskWorktreeLocation(repoRoot, worktreeName, debugProbeOverride) {
    return this.call("taskWorkflowResolveTaskWorktreeLocation", repoRoot, worktreeName, debugProbeOverride);
  }

  taskWorkflowResolveMainSeedMirrorLocation(repoRoot, seedName) {
    return this.call("taskWorkflowResolveMainSeedMirrorLocation", repoRoot, seedName);
  }

  taskWorkflowEnsureMainSeedMirror(repoRoot, options) {
    return this.call("taskWorkflowEnsureMainSeedMirror", repoRoot, options);
  }

  taskWorkflowEnsureStatusManifest(repoRoot, snapshotId) {
    return this.call("taskWorkflowEnsureStatusManifest", repoRoot, snapshotId);
  }

  taskWorkflowQueueSummary(repoRoot, remoteName) {
    return this.call("taskWorkflowQueueSummary", repoRoot, remoteName);
  }

  taskWorkflowTaskAudit(repoRoot, taskId, scope) {
    return this.call("taskWorkflowTaskAudit", repoRoot, taskId, scope);
  }

  taskWorkflowTaskAbandon(repoRoot, taskId, scope) {
    return this.call("taskWorkflowTaskAbandon", repoRoot, taskId, scope);
  }

  taskWorkflowTaskLand(repoRoot, taskOrChangeId, scope) {
    return this.call("taskWorkflowTaskLand", repoRoot, taskOrChangeId, scope);
  }

  taskWorkflowLandSubmit(repoRoot, changeId, options) {
    return this.call("taskWorkflowLandSubmit", repoRoot, changeId, options);
  }

  taskWorkflowLandShow(repoRoot, submissionId, options) {
    return this.call("taskWorkflowLandShow", repoRoot, submissionId, options);
  }

  taskWorkflowLandRetry(repoRoot, submissionId, options) {
    return this.call("taskWorkflowLandRetry", repoRoot, submissionId, options);
  }

  buildLinkedTaskLookupPayload(taskLinksByItemRows, tasksByPlanRows) {
    return this.call("buildLinkedTaskLookupPayload", taskLinksByItemRows, tasksByPlanRows);
  }

  buildLinkedChangeLookupPayload(changeLinksByTaskRows) {
    return this.call("buildLinkedChangeLookupPayload", changeLinksByTaskRows);
  }

  buildTaskTrackingTitlePayload(task) {
    return this.call("buildTaskTrackingTitlePayload", task);
  }

  buildTaskTrackingMetadataPayload(task, authorMode, trackingPolicy) {
    return this.call("buildTaskTrackingMetadataPayload", task, authorMode, trackingPolicy);
  }

  buildTaskAuditVerdictPayload(task, changeRows, targetLine) {
    return this.call("buildTaskAuditVerdictPayload", task, changeRows, targetLine);
  }

  workflowLandedFacts(state) {
    return this.call("workflowLandedFacts", state);
  }

  workflowLandFullFacts(state) {
    return this.call("workflowLandFullFacts", state);
  }

  workflowReadyFacts(state) {
    return this.call("workflowReadyFacts", state);
  }

  workflowLandPhaseFacts(state, readyState) {
    return this.call("workflowLandPhaseFacts", state, readyState);
  }

  projectWorkflowLandPhaseReadModel(state, readyState) {
    return this.call("projectWorkflowLandPhaseReadModel", state, readyState);
  }

  workflowRemoteMutationReceipt(input) {
    return this.call("workflowRemoteMutationReceipt", input);
  }

  workflowRemoteActionMutationReceipts(code, result) {
    return this.call("workflowRemoteActionMutationReceipts", code, result);
  }

  workflowApplyPhasePayload(phase, code, options) {
    return this.call("workflowApplyPhasePayload", phase, code, options);
  }

  workflowApplyPhaseSummary(applyPhase) {
    return this.call("workflowApplyPhaseSummary", applyPhase);
  }

  workflowMutationReceiptSummary(receipt) {
    return this.call("workflowMutationReceiptSummary", receipt);
  }

  workflowAppliedActionSummary(action) {
    return this.call("workflowAppliedActionSummary", action);
  }

  workflowReadyPayloadDirect(repoRoot, changeId, remoteName) {
    return this.call("workflowReadyPayloadDirect", repoRoot, changeId, remoteName);
  }

  workflowLandPayloadDirect(repoRoot, changeId, remoteName) {
    return this.call("workflowLandPayloadDirect", repoRoot, changeId, remoteName);
  }

  workflowReadyApplyDirect(repoRoot, changeId, options) {
    return this.call("workflowReadyApplyDirect", repoRoot, changeId, options);
  }

  workflowLandApplyDirect(repoRoot, changeId, options) {
    return this.call("workflowLandApplyDirect", repoRoot, changeId, options);
  }

  projectWorkflowLandedReadModel(context, state) {
    return this.call("projectWorkflowLandedReadModel", context, state);
  }

  projectWorkflowLandFullReadModel(context, state, applyOwnedContinuation) {
    return this.call("projectWorkflowLandFullReadModel", context, state, applyOwnedContinuation);
  }

  projectWorkflowReadyReadModel(context, state, ignoreWorkspaceAuthoring, patchsetIsAuthoritative, applyOwnedContinuation) {
    return this.call("projectWorkflowReadyReadModel", context, state, ignoreWorkspaceAuthoring, patchsetIsAuthoritative, applyOwnedContinuation);
  }

  createWorkflowRelease(repoRoot, input) {
    return this.call("createWorkflowRelease", repoRoot, input);
  }

  listWorkflowReleases(repoRoot) {
    return this.call("listWorkflowReleases", repoRoot);
  }

  getWorkflowRelease(repoRoot, releaseId) {
    return this.call("getWorkflowRelease", repoRoot, releaseId);
  }

  updateWorkflowRelease(repoRoot, releaseId, options) {
    return this.call("updateWorkflowRelease", repoRoot, releaseId, options);
  }

  taskWorkflowReleaseCandidateCreate(repoRoot, version, line, profile) {
    return this.call("taskWorkflowReleaseCandidateCreate", repoRoot, version, line, profile);
  }

  taskWorkflowReleaseCheck(repoRoot, releaseId, options) {
    return this.call("taskWorkflowReleaseCheck", repoRoot, releaseId, options);
  }

  taskWorkflowReleaseBuild(repoRoot, releaseId) {
    return this.call("taskWorkflowReleaseBuild", repoRoot, releaseId);
  }

  taskWorkflowReleaseFormula(repoRoot, releaseId, name) {
    return this.call("taskWorkflowReleaseFormula", repoRoot, releaseId, name);
  }

  taskWorkflowReleaseShow(repoRoot, releaseId, remoteName) {
    return this.call("taskWorkflowReleaseShow", repoRoot, releaseId, remoteName);
  }

  taskWorkflowReleasePublish(repoRoot, releaseId, remoteName) {
    return this.call("taskWorkflowReleasePublish", repoRoot, releaseId, remoteName);
  }

  releaseAdapterCheck(repoRoot, version, options) {
    return this.call("releaseAdapterCheck", repoRoot, version, options);
  }

  releaseAdapterBuild(repoRoot, version, options) {
    return this.call("releaseAdapterBuild", repoRoot, version, options);
  }

  releaseCandidateCreate(repoRoot, version, options) {
    return this.call("releaseCandidateCreate", repoRoot, version, options);
  }

  releaseCheck(repoRoot, releaseId, options) {
    return this.call("releaseCheck", repoRoot, releaseId, options);
  }

  releaseBuild(repoRoot, releaseId, options) {
    return this.call("releaseBuild", repoRoot, releaseId, options);
  }

  releaseNativeSource(repoRoot, request) {
    return this.call("releaseNativeSource", repoRoot, request);
  }

  releaseNativeBundle(repoRoot, releaseId, nativeMatrixDir) {
    return this.call("releaseNativeBundle", repoRoot, releaseId, nativeMatrixDir);
  }

  releasePackage(repoRoot, releaseId, channel, publicSourceRoot) {
    return this.call("releasePackage", repoRoot, releaseId, channel, publicSourceRoot);
  }

  releaseFormula(repoRoot, releaseId, name, pythonFormula) {
    return this.call("releaseFormula", repoRoot, releaseId, name, pythonFormula);
  }

  releaseShow(repoRoot, releaseId, options) {
    return this.call("releaseShow", repoRoot, releaseId, options);
  }

  releasePublish(repoRoot, releaseId, remoteName) {
    return this.call("releasePublish", repoRoot, releaseId, remoteName);
  }

  releasePromote(repoRoot, releaseId, channel, publicSourceRoot) {
    return this.call("releasePromote", repoRoot, releaseId, channel, publicSourceRoot);
  }

  formatRelease(payload) {
    return this.call("formatRelease", payload);
  }

  externalStatus(repoRoot) {
    return this.call("externalStatus", repoRoot);
  }

  externalDoctor(repoRoot, failOnBlocking) {
    return this.call("externalDoctor", repoRoot, failOnBlocking);
  }

  externalUpdate(repoRoot, options) {
    return this.call("externalUpdate", repoRoot, options);
  }

  externalLink(repoRoot, name, path) {
    return this.call("externalLink", repoRoot, name, path);
  }

  externalUnlink(repoRoot, name) {
    return this.call("externalUnlink", repoRoot, name);
  }

  teamContacts(repoRoot) {
    return this.call("teamContacts", repoRoot);
  }

  teamSessionEnsure(repoRoot, agent, role) {
    return this.call("teamSessionEnsure", repoRoot, agent, role);
  }

  teamSessionAcknowledge(repoRoot, activation) {
    return this.call("teamSessionAcknowledge", repoRoot, activation);
  }

  formatTeamContacts(payload) {
    return this.call("formatTeamContacts", payload);
  }

  formatTeamSessionEnsure(payload, agent) {
    return this.call("formatTeamSessionEnsure", payload, agent);
  }

  formatTeamSessionAcknowledge(payload) {
    return this.call("formatTeamSessionAcknowledge", payload);
  }

  binaryDbUpgradeU64Seconds(repoRoot, generationRoot, sourceTimeWidth, activate) {
    return this.call("binaryDbUpgradeU64Seconds", repoRoot, generationRoot, sourceTimeWidth, activate);
  }

  binaryDbAgentPool50(repoRoot, generationRoot, activate) {
    return this.call("binaryDbAgentPool50", repoRoot, generationRoot, activate);
  }

  formatBinaryDbUpgradeU64Seconds(payload) {
    return this.call("formatBinaryDbUpgradeU64Seconds", payload);
  }

  formatBinaryDbAgentPool50(payload) {
    return this.call("formatBinaryDbAgentPool50", payload);
  }

  formatExternalStatus(payload) {
    return this.call("formatExternalStatus", payload);
  }

  formatExternalDoctor(payload) {
    return this.call("formatExternalDoctor", payload);
  }

  formatExternalUpdate(payload) {
    return this.call("formatExternalUpdate", payload);
  }

  formatExternalLink(payload) {
    return this.call("formatExternalLink", payload);
  }

  formatExternalUnlink(payload) {
    return this.call("formatExternalUnlink", payload);
  }

  remoteRecoverHead(repoRoot, options) {
    return this.call("remoteRecoverHead", repoRoot, options);
  }

  formatRemoteRecoverHead(payload) {
    return this.call("formatRemoteRecoverHead", payload);
  }

  parseSnapshotAuthoringReference(reference) {
    return this.call("parseSnapshotAuthoringReference", reference);
  }

  workflowIdFamilies() {
    return this.call("workflowIdFamilies");
  }

  workflowTaskChangeOriginNamespacePrefixes() {
    return this.call("workflowTaskChangeOriginNamespacePrefixes");
  }

  policyRequirementFlags() {
    return this.call("policyRequirementFlags");
  }

  agentCapabilities() {
    return parseObjectPayload(
      this.call("agentWorkerCapabilitiesJson"),
      "ait-agent-worker capabilities",
    );
  }

  agentManagement(request) {
    return parseJsonPayload(
      this.call("agentManagementJson", encodeRequest(request, "ait-agent")),
      "ait-agent response",
    );
  }

  agentWorkerTransaction(request) {
    return parseJsonPayload(
      this.call(
        "agentWorkerTransactionJson",
        encodeRequest(request, "ait-agent-worker"),
      ),
      "ait-agent-worker response",
    );
  }
}

export function requiredAddonExports() {
  return [...REQUIRED_EXPORTS];
}

function loadContract() {
  if (validatedContract !== null) {
    return validatedContract;
  }
  const contract = readJson(CONTRACT_PATH, "npm addon contract");
  assertExactKeys(contract, CONTRACT_KEYS, "npm addon contract");
  if (
    contract.schema !== "ait.node.napi-platform-packages/v2" ||
    contract.top_level_package !== "@wa120/ait-native" ||
    contract.family_version !== "1.2.0" ||
    !Array.isArray(contract.payloads) ||
    contract.payloads.length !== 6
  ) {
    throw new NativeResolutionError("npm addon contract identity is invalid");
  }
  const packages = new Set();
  const targets = new Set();
  for (const [index, payload] of contract.payloads.entries()) {
    assertExactKeys(payload, PAYLOAD_KEYS, `npm addon contract row ${index}`);
    const platform = TARGETS.get(payload.target);
    const expectedPackage =
      platform === undefined
        ? null
        : `@wa120/ait-native-${platform.os}-${platform.cpu}`;
    if (
      platform === undefined ||
      payload.os !== platform.os ||
      payload.cpu !== platform.cpu ||
      payload.libc !== platform.libc ||
      payload.package !== expectedPackage ||
      payload.component !== "ait-node" ||
      payload.binding_repository !== "ait-core" ||
      payload.binding_snapshot !== "SNP-D0E55D404426" ||
      payload.license !== "Apache-2.0" ||
      payload.version !== contract.family_version ||
      payload.addon !== "native/ait_napi.node"
    ) {
      throw new NativeResolutionError(
        `npm addon contract row ${index} has invalid authority`,
      );
    }
    if (packages.has(payload.package) || targets.has(payload.target)) {
      throw new NativeResolutionError(
        `npm addon contract row ${index} duplicates a package or target`,
      );
    }
    packages.add(payload.package);
    targets.add(payload.target);
  }
  validatedContract = contract;
  return contract;
}

export function detectRuntimeLibc(
  platform = process.platform,
  report = runtimeReport(),
) {
  if (platform !== "linux") {
    return null;
  }
  const glibcVersion = report?.header?.glibcVersionRuntime;
  if (typeof glibcVersion === "string" && glibcVersion.length > 0) {
    return "glibc";
  }
  const sharedObjects = report?.sharedObjects;
  if (
    Array.isArray(sharedObjects) &&
    sharedObjects.some(
      (entry) =>
        typeof entry === "string" &&
        /(?:^|[\\/])(?:ld-musl-[^\\/]+|libc\.musl-[^\\/]+)\.so(?:\.\d+)*$/.test(
          entry,
        ),
    )
  ) {
    return "musl";
  }
  return "unknown";
}

export function selectPlatformPayload(
  contract,
  platform = {
    os: process.platform,
    cpu: process.arch,
    libc: detectRuntimeLibc(),
  },
) {
  const matches = contract.payloads.filter(
    (payload) =>
      payload.os === platform.os &&
      payload.cpu === platform.cpu &&
      payload.libc === platform.libc,
  );
  if (matches.length !== 1) {
    const libc = platform.os === "linux" ? `/${platform.libc}` : "";
    throw new NativeResolutionError(
      `ait-native does not support Node-API on ${platform.os}/${platform.cpu}${libc}`,
    );
  }
  return matches[0];
}

function runtimeReport() {
  try {
    return process.report?.getReport?.() ?? null;
  } catch {
    return null;
  }
}

function validatePlatformPackage(packageJson, payload) {
  const metadata = packageJson.aitNativeAddon;
  assertExactKeys(metadata, ADDON_METADATA_KEYS, payload.package);
  const libcMatches =
    payload.libc === null
      ? packageJson.libc === undefined
      : Array.isArray(packageJson.libc) &&
        packageJson.libc.length === 1 &&
        packageJson.libc[0] === payload.libc;
  if (
    packageJson.name !== payload.package ||
    packageJson.version !== payload.version ||
    packageJson.license !== payload.license ||
    packageJson.main !== payload.addon ||
    !Array.isArray(packageJson.os) ||
    packageJson.os.length !== 1 ||
    packageJson.os[0] !== payload.os ||
    !Array.isArray(packageJson.cpu) ||
    packageJson.cpu.length !== 1 ||
    packageJson.cpu[0] !== payload.cpu ||
    !libcMatches ||
    metadata.schema !== "ait.node.napi-platform-addon/v2" ||
    metadata.component !== payload.component ||
    metadata.target !== payload.target ||
    metadata.libc !== payload.libc ||
    metadata.addon !== payload.addon ||
    metadata.binding_repository !== payload.binding_repository ||
    metadata.binding_snapshot !== payload.binding_snapshot ||
    packageJson.dependencies !== undefined ||
    packageJson.optionalDependencies !== undefined ||
    packageJson.scripts !== undefined ||
    packageJson.bin !== undefined
  ) {
    throw new NativeResolutionError(
      `Rust AIT binding package ${payload.package} is invalid`,
    );
  }
}

function readJson(filePath, label) {
  try {
    return JSON.parse(readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new NativeResolutionError(`${label} is not valid JSON`, {
      cause: error,
    });
  }
}

function assertExactKeys(value, expectedKeys, label) {
  if (!isRecord(value)) {
    throw new NativeResolutionError(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...expectedKeys].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new NativeResolutionError(
      `${label} fields must be exactly ${expected.join(", ")}`,
    );
  }
}

function normalizeAddonPath(value) {
  const candidate =
    value instanceof URL ? fileURLToPath(value) : String(value).trim();
  if (candidate.length === 0) {
    throw new NativeResolutionError("native addon path must not be empty");
  }
  return path.resolve(candidate);
}

function assertRegularAddon(addonPath) {
  let entry;
  try {
    entry = lstatSync(addonPath);
  } catch (error) {
    throw new NativeResolutionError(
      `Rust AIT binding is unavailable because ${addonPath} could not be inspected`,
      { cause: error },
    );
  }
  if (entry.isSymbolicLink() || !entry.isFile()) {
    throw new NativeResolutionError(
      `Rust AIT binding must be a regular non-symlink file: ${addonPath}`,
    );
  }
}

function regularNonSymlinkFile(filePath) {
  try {
    const entry = lstatSync(filePath);
    return entry.isFile() && !entry.isSymbolicLink();
  } catch (error) {
    if (error?.code === "ENOENT") {
      return false;
    }
    throw error;
  }
}

function encodeRequest(request, label) {
  if (!isRecord(request)) {
    throw new TypeError(`${label} request must be an object`);
  }
  try {
    return JSON.stringify(request);
  } catch (error) {
    throw new TypeError(`${label} request is not JSON-serializable`, {
      cause: error,
    });
  }
}

function parseJsonPayload(payload, label) {
  if (typeof payload !== "string") {
    throw new NativeProtocolError(`${label} must be JSON text`);
  }
  try {
    return JSON.parse(payload);
  } catch (error) {
    throw new NativeProtocolError(`${label} is invalid JSON`, {
      cause: error,
    });
  }
}

function parseObjectPayload(payload, label) {
  const value = parseJsonPayload(payload, label);
  if (!isRecord(value)) {
    throw new NativeProtocolError(`${label} must be a JSON object`);
  }
  return value;
}

function requiredText(payload, field, label) {
  const value = payload[field];
  if (typeof value !== "string" || value.length === 0) {
    throw new NativeProtocolError(
      `${label} field ${field} must be non-empty text`,
    );
  }
  return value;
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
