export interface ProjectInfo {
  path: string;
  title: string;
  methodology: string;
  coders: string[];
  last_saved_by: string | null;
  last_saved_at: string | null;
  merge_same_speaker?: boolean;
}

export interface InterviewRosterEntry {
  id?: string;
  project_id?: string;
  study_label: string;
  segment_count: number;
  content_hash?: string | null;
  interview_content_hash?: string | null;
  participant_label?: string;
  revision?: number;
  deleted?: boolean;
  updated_at?: string | null;
}

export interface Code {
  id: string;
  name: string;
  definition: string | null;
  inclusion_criteria: string | null;
  exclusion_criteria: string | null;
  example: string | null;
  parent_id: string | null;
  color: string;
  sort_order: number;
  is_retired: boolean;
  /** Distinct passages carrying this code, across all interviews and coders. */
  usage_count: number;
}

/** Retiring keeps existing coding intact; purging strips the code from it. */
export type DeleteCodeMode = "retire" | "purge";

export interface DeleteCodeResult {
  mode: DeleteCodeMode;
  code_name: string;
  segments_updated: number;
  coded_segments_removed: number;
}

export interface InterviewDeleteImpact {
  participant_label: string;
  segment_count: number;
  coded_segment_count: number;
  has_hub_memo: boolean;
}

export interface Interview {
  id: string;
  participant_label: string;
  interview_date: string | null;
  modality: string | null;
  diagnosis_notes: string | null;
  interviewers: string[];
  hub_memo: string | null;
  audio_path: string | null;
  segment_count: number;
  /**
   * Passages the other coder reports for this interview. Non-null with a local
   * `segment_count` of 0 means it arrived through sync and its transcript is
   * still in the shared folder.
   */
  remote_segment_count: number | null;
}

export interface TranscriptSegment {
  id: string;
  interview_id: string;
  segment_index: number;
  speaker: string;
  timestamp_start: string;
  timestamp_end: string | null;
  text: string;
  block_id: string | null;
  section_tag: string | null;
}

export interface CodedSegment {
  id: string;
  interview_id: string;
  segment_id: string;
  code_ids: string[];
  coder_name: string;
  memo: string | null;
  char_start: number | null;
  char_end: number | null;
  quote_text: string;
  block_id: string | null;
  timestamp_start: string;
  participant_label: string;
}

export interface ChangedCodingEdge {
  interview_id: string;
  segment_id: string;
  code_id: string;
  char_start: number | null;
  char_end: number | null;
  present: boolean;
}

export interface EnsureCodeAndApplyResult {
  code: Code;
  coded_segment: CodedSegment;
  created: boolean;
  changed_edges: ChangedCodingEdge[];
}

export interface MutateCodingEdgeResult {
  coded_segment: CodedSegment | null;
  changed_edge: ChangedCodingEdge;
}

export interface ActivityLogEntry {
  id: string;
  coder_name: string;
  action: string;
  detail: string | null;
  created_at: string;
}

export interface ExportResult {
  exports_dir: string;
  coded_segment_count: number;
  interview_file_count: number;
  unresolved_conflict_count: number;
  files: string[];
  exported_at: string;
  exported_by: string;
}

export type UpdatePhase =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "readyToInstall"
  | "preparing"
  | "installing"
  | "failed";

export interface UpdateFailure {
  stage: string;
  retryable: boolean;
  message: string;
}

export interface UpdateCoordinatorStatus {
  phase: UpdatePhase;
  currentVersion: string;
  targetVersion: string | null;
  downloadedBytes: number;
  totalBytes: number | null;
  lastCheckedAt: string | null;
  syncPreflightOutcome: "synced" | "offline_or_failed" | "timed_out" | null;
  failure: UpdateFailure | null;
}

export interface WorkspaceState {
  active_interview_id: string | null;
  selected_segment_id: string | null;
  active_coder: string | null;
}

export interface RecentProject {
  path: string;
  title: string;
  last_opened_at: string;
  group_id?: string;
  group_title?: string;
  coder_name?: string;
  former_group_id?: string;
  former_group_title?: string;
  readiness?: import("./home-rows").StudyReadiness;
}

export interface PanelWidths {
  codebook: number;
  memos: number;
}

export interface TableReceipt {
  applied: number;
  superseded: number;
  deferred: number;
}

/** What one sync run did. */
export interface SyncOutcome {
  pushedCoded: number;
  pushedCodes: number;
  pulledCoded: number;
  pulledCodes: number;
  pulledInterviews: number;
  /** Transcripts this machine still needs from the shared folder. */
  missingTranscripts: MissingTranscript[];
  /** Codes the other coder added, so they don't appear silently. */
  newCodeNames: string[];
  syncedAt: string;
  codedReceipt: TableReceipt;
  codesReceipt: TableReceipt;
  interviewsReceipt: TableReceipt;
  truncated: boolean;
}

export interface MissingTranscript {
  studyLabel: string;
  segmentCount: number;
  /** The interview exists here, but was imported from a different file. */
  mismatched: boolean;
  remoteContentHash?: string | null;
}

/** What redeeming a legacy invitation settled. */
export interface Redeemed {
  projectId: string;
  /** The name the inviter assigned — not one the joiner chooses. */
  coderName: string;
}

/** One group the signed-in account belongs to, with the name it files under. */
export interface MembershipSummary {
  projectId: string;
  title: string;
  coderName: string;
  members: string[];
  role: "admin" | "coder" | string;
}

export interface MembershipsCache {
  memberships: MembershipSummary[];
  cachedAt?: string;
}

/** What joining a group settled. */
export interface JoinedGroup {
  projectId: string;
  title: string;
  /**
   * The name the server recorded. On a rejoin it is the *existing*
   * membership's name, not the one typed this time.
   */
  coderName: string;
  /** False when this account was already a member — nothing changed. */
  created: boolean;
}

/** What a freshly created group came back with. */
export interface CreatedGroup {
  projectId: string;
  /** Display-formatted (`XXXX-XXXX`), ready to show and share. */
  groupKey: string;
}

/** One person in the group, as the roster shows them. */
export interface GroupMember {
  coderName: string;
  /**
   * When they joined. Null for rows of coding filed under a name no current
   * member holds — their work outlives the membership bookkeeping.
   */
  joinedAt: string | null;
  /** Latest activity across their coding. Null if they have not filed any. */
  lastActiveAt: string | null;
  /** Live rows of coding under this name. */
  codedCount: number;
  /** The signed-in account's own row — the one the rename pencil belongs on. */
  isYou: boolean;
  role?: "admin" | "coder" | string;
  userId?: string;
}

/** The group's face: its key, its people, and what they have done. */
export interface GroupInfo {
  title: string;
  /** Display-formatted (`XXXX-XXXX`). */
  groupKey: string;
  members: GroupMember[];
}

export interface SyncV2ReadinessMember {
  userId: string;
  coderName: string;
  role: string;
  ready: boolean;
  readyAt: string | null;
  lastDeviceIdSuffix: string | null;
}

export interface SyncV2Readiness {
  protocol: 1 | 2;
  generationSuffix: string | null;
  head: number;
  members: SyncV2ReadinessMember[];
}

export interface SyncV2Activation {
  protocol: 2;
  generationSuffix: string;
  head: number;
  legacyActorRows: number;
}

/** The settled name after a rename, plus the name it replaced. */
export interface RenamedSelf {
  coderName: string;
  previousName: string;
}

export interface SyncStatus {
  /** A server is configured on this machine. */
  configured: boolean;
  /** Signed in this session, or restored from the session store. */
  signedIn: boolean;
  /** Email on the account, when the server sent one. */
  signedInEmail: string | null;
  /**
   * This build carries a compiled-in server, so a fresh install already knows
   * where to sync. What decides whether an invitation is six characters or a
   * whole block.
   */
  serverPreset: boolean;
  /** The open project's sync identity, minted on demand. */
  projectId: string | null;
  /**
   * This folder is a member of a remote group — not merely holding a locally
   * minted id.
   */
  inGroup: boolean;
  /** Whether the current signed-in account is an admin in this group. */
  isGroupAdmin: boolean;
  lastSyncedAt: string | null;
  /** Local changes not yet pushed. */
  pendingChanges: number;
  neverSynced?: boolean;
  realtimeConnected?: boolean;
  realtimeHealth?: "off" | "connecting" | "connected" | "unavailable";
  serverSchemaVersion?: number | null;
  requiredServerSchema?: number;
  pendingUnbindOperation?: "leave" | "delete" | null;
  coordinatorRunning?: boolean;
  coordinatorRerunRequested?: boolean;
  coordinatorBackoffAttempt?: number;
  coordinatorLastTrigger?: string | null;
  oldestOutboxAgeSeconds?: number | null;
  protocol?: number;
  generationSuffix?: string | null;
  deviceIdSuffix?: string | null;
  localSequence?: number;
  observedHead?: number;
  outboxCount?: number;
  blockedOutboxCount?: number;
  unresolvedConflictCount?: number;
  lastRealtimeAt?: string | null;
  lastSuccessAt?: string | null;
  sequenceLagAgeSeconds?: number | null;
}

export interface DiagnosticsReport {
  localCodedCount: number;
  localCodeCount: number;
  remoteCodedCount: number;
  remoteCodeCount: number;
  missingRemoteCodedCount: number;
  missingRemoteCoderNames: string[];
  pendingToSendCount: number;
  clockSkewSeconds: number | null;
  summaryMessage: string;
  rawCodedCursor: string | null;
  rawCodebookCursor: string | null;
  rawInterviewCursor: string | null;
  lastSyncedAt: string | null;
  needsRepair: boolean;
  needsSend: boolean;
  lastError: string | null;
  lastOutcome: SyncOutcome | null;
}

export interface PresenceUser {
  coderName: string;
  participantLabel: string;
  updatedAt: string;
}

export interface AppPreferences {
  reopen_last_project: boolean;
  signin_prompt_seen?: boolean;
  /** v1.2 intent-first onboarding choice; migrates over signin_prompt_seen. */
  onboarding_choice_seen?: boolean;
  /** Quiet update checks (default true). False stops startup+interval checks. */
  automatic_update_checks?: boolean;
  last_guide_section_id: string | null;
  panel_widths: PanelWidths | null;
  coach_dismissed: boolean;
  /**
   * Merge consecutive same-speaker turns into one passage when importing.
   * Read at parse time; already-imported transcripts are unaffected.
   */
  merge_same_speaker: boolean;
  /** "light" | "dark" | "system". Null or absent means light. */
  theme: string | null;
  /** Project path → which coder this machine is on that project. */
  coder_identities: Record<string, string>;
  /**
   * Sync server for this installation. Per machine, not per project: where
   * this laptop syncs is a property of the laptop.
   */
  sync_url: string | null;
  sync_anon_key: string | null;
  /**
   * Passage-text zoom as a scale factor (1 = 100%). Null/absent = default.
   * Local-only reading preference: never synced, never exported.
   */
  transcript_zoom?: number | null;
  /** Whether the left codebook rail is collapsed. Local-only, like widths. */
  codebook_collapsed?: boolean;
  /**
   * Per-interview speaker-redaction toggle, keyed by interview id.
   * Local-only view/export layer; never enters a sync payload.
   */
  speaker_redaction?: Record<string, boolean>;
}

export interface AppVersionInfo {
  name: string;
  version: string;
  copyright: string | null;
  /** Full source SHA, or the literal "development" for local builds. */
  build_commit?: string | null;
  source_url?: string | null;
  release_url?: string | null;
  install_guide_url?: string | null;
}

export interface ProjectFileEntry {
  path: string;
  relative_path: string;
  name: string;
  size: number;
  modified_at: string | null;
  kind: string;
}

export interface StatusMessageAction {
  label: string;
  onClick: () => void;
}

export interface StatusMessage {
  type: "success" | "error" | "info";
  text: string;
  action?: StatusMessageAction;
  durationMs?: number;
}

export type ToastAction = StatusMessageAction;

export interface Toast {
  id: string;
  type: "success" | "info" | "error";
  text: string;
  action?: ToastAction;
  durationMs?: number;
}

export interface SegmentSpeakerChange {
  segment_id: string;
  old_speaker: string;
  new_speaker: string;
}

export interface InterviewSpeakerSummary {
  speaker: string;
  turn_count: number;
}

export interface OpenStage {
  step: number;
  total: number;
  label: string;
}

export interface ParsedVttCue {
  speaker: string;
  timestamp_start: string;
  timestamp_end: string | null;
  text: string;
}

export interface SegmentInput {
  speaker: string;
  timestamp_start: string;
  timestamp_end: string | null;
  text: string;
  section_tag: string | null;
}

export interface ClearWorkspaceInput {
  interview_id: string | null;
  clear_hub_memos: boolean;
  clear_activity_log: boolean;
  coder_name: string;
}

export interface ClearWorkspaceResult {
  cleared_coded_segments: number;
  cleared_block_ids: number;
  cleared_hub_memos: number;
  cleared_activity_log: boolean;
  scope: string;
}

// ── Backups ──────────────────────────────────────────────────────────────────

/** Why a snapshot was taken. Automatic ones roll; the other two are kept. */
export type BackupReason = "automatic" | "manual" | "pre-restore";

/**
 * One snapshot. The counts are what the project held when it was taken, not
 * what it holds now — they are the only way to tell two snapshots apart.
 */
export interface BackupInfo {
  path: string;
  file_name: string;
  size_bytes: number;
  created_at: string;
  reason: BackupReason;
  note: string | null;
  app_version: string;
  schema_version: number;
  project_title: string;
  codes: number;
  interviews: number;
  segments: number;
  coded_segments: number;
}

export interface RestoreOutcome {
  restored_from: string;
  restored: Omit<BackupInfo, "path" | "file_name" | "size_bytes">;
  /** The snapshot taken of the pre-restore state, so a restore is undoable. */
  safety_backup_path: string;
}

export interface DiagnosticCounts {
  codes: number;
  interviews: number;
  segments: number;
  coded_segments: number;
}

export interface DiagnosticTimings {
  connection: number;
  schema: number;
  snapshot_queries: number;
  total: number;
}

export interface ProjectOpenDiagnostics {
  schema_version: number;
  counts: DiagnosticCounts;
  timings_ms: DiagnosticTimings;
}

export interface ProjectOpenSnapshot {
  project: ProjectInfo;
  codes: Code[];
  interviews: Interview[];
  workspace: WorkspaceState;
  active_interview_id: string | null;
  selected_segment_id: string | null;
  segments: TranscriptSegment[];
  coded_segments: CodedSegment[];
  total_coded_count: number;
  recent_code_ids: string[];
  reviewed_segment_ids: string[];
  diagnostics: ProjectOpenDiagnostics;
  /** Local-study UUID this open is bound to (recovery scope). Additive. */
  project_key?: string | null;
  /** Backend workspace epoch for checked note writes. Additive. */
  workspace_epoch?: string | null;
}

export interface SyncConflictSummary {
  id: string;
  entity_type: string;
  entity_id: string;
  field_name: string;
  status: string;
  created_at: string;
}

export interface SyncConflictDetail {
  id: string;
  entity_type: "code" | "interview" | "coding" | string;
  entity_label: string;
  field_name: string;
  current_value: unknown;
  proposed_value: unknown;
  proposer_label: string | null;
  status: "unresolved" | "resolved" | string;
  created_at: string;
}

export interface LiveWorkspaceSyncStatus {
  protocol: number;
  generation: string | null;
  local_sequence: number;
  observed_head: number;
  outbox_count: number;
  blocked_count: number;
  unresolved_conflict_count: number;
}

export interface LiveWorkspaceSnapshot {
  project: ProjectInfo;
  interviews: Interview[];
  codes: Code[];
  retired_codes: Code[];
  active_interview_id: string | null;
  selected_segment_id: string | null;
  segments: TranscriptSegment[];
  coded_segments: CodedSegment[];
  pending_coded_count: number;
  coded_count: number;
  conflicts: SyncConflictSummary[];
  sync_status: LiveWorkspaceSyncStatus;
  local_revision: number;
  reviewed_segment_ids: string[];
  /** Local-study UUID this open is bound to (recovery scope). Additive. */
  project_key?: string | null;
  /** Current backend workspace epoch for checked note writes. Additive. */
  workspace_epoch?: string | null;
}

export type JoinTargetVerdict =
  | { verdict: "already_set_up_here"; path: string }
  | { verdict: "ready"; path: string }
  | { verdict: "reconnectable"; path: string }
  | { verdict: "adoptable_unbound"; path: string }
  | { verdict: "bound_elsewhere"; path: string; suggested_name: string; suggested_path: string }
  | { verdict: "unrelated_collision"; path: string; suggested_name: string; suggested_path: string }
  | { verdict: "occupied"; path: string; suggested_name: string; suggested_path: string }
  | { verdict: "available"; suggested_name: string; suggested_path: string }
  | { verdict: "unavailable"; path: string; reason: string }
  | { verdict: "ambiguous"; paths: string[] }
  | { verdict: "invalid"; path: string; reason: string };

export interface ProjectDeletionSummary {
  interview_count: number;
  coded_segment_count: number;
  memo_count: number;
  segment_count?: number;
  hub_memo_count?: number;
  passage_memo_count?: number;
  unsynced_op_count?: number;
  conflict_count?: number;
  recovery_draft_count?: number;
  local_attachment_count?: number;
  unreadable_sections?: string[];
  is_completely_read?: boolean;
}

export type StudyLocation =
  | { state: "reachable" }
  | { state: "volume_not_mounted"; volume_name: string }
  | { state: "cloud_not_downloaded"; provider: string }
  | { state: "cloud_provider_absent"; provider: string }
  | { state: "permission_denied" }
  | { state: "gone" };

export type RelinkOutcome =
  | { outcome: "exact_match"; new_path: string; message: string }
  | { outcome: "name_match_only"; candidate_path: string; message: string }
  | { outcome: "not_found" };

export type OpenMarkerStatus =
  | { status: "clear" }
  | { status: "active_other_machine"; machineName: string; coderName: string; minutesAgo: number };

export interface LeftStudy {
  projectId: string;
  title: string;
  groupKey: string;
  coderName: string;
  leftAt: string;
}

// ── Note drafts: local crash recovery + checked writes ──────────────────────
//
// Recovery drafts stay on this computer. They are not committed notes, never
// sync, and never enter exports or study backups. Mirrors the Rust DTOs in
// src-tauri/src/models.rs (snake_case wire format).

export type NoteDraftKind = "coding" | "interview";

/** One unfinished note generation in app-local recovery. */
export interface NoteDraftRecord {
  draft_id: string;
  project_key: string;
  kind: string;
  target_id: string;
  interview_id: string;
  coder_name: string | null;
  base_text: string;
  draft_text: string;
  revision: number;
  participant_label: string;
  segment_id: string | null;
  segment_index: number | null;
  char_start: number | null;
  char_end: number | null;
  quote_text: string | null;
  updated_at: string;
  discarded: boolean;
  /** Study title at list time (JOINed). Absent on single-record reads. */
  project_title?: string | null;
}

export interface BeginNoteDraftInput {
  project_key: string;
  epoch: string;
  kind: NoteDraftKind;
  target_id: string;
  interview_id: string;
  coder_name?: string | null;
  participant_label: string;
  segment_id?: string | null;
  segment_index?: number | null;
  char_start?: number | null;
  char_end?: number | null;
  quote_text?: string | null;
}

export type BeginNoteDraftResult =
  | { status: "active"; record: NoteDraftRecord; committed_text: string }
  | { status: "missing-target" }
  | { status: "stale-workspace" };

export interface PutNoteDraftInput {
  project_key: string;
  epoch: string;
  draft_id: string;
  expected_revision: number;
  draft_text: string;
}

export type PutNoteDraftResult =
  | { status: "stored"; record: NoteDraftRecord }
  | { status: "revision-mismatch"; record: NoteDraftRecord }
  | { status: "stale-generation" }
  | { status: "stale-workspace" };

export interface DiscardNoteDraftInput {
  draft_id: string;
}

export interface DiscardNoteDraftResult {
  draft_id: string;
  revision: number;
}

export interface SaveNoteDraftInput {
  project_key: string;
  epoch: string;
  draft_id: string;
  revision: number;
  kind: NoteDraftKind;
  target_id: string;
  expected_saved_text: string;
  draft_text: string;
}

/** Typed commit outcome. IO failures reject; content outcomes resolve. */
export type SaveNoteDraftResult =
  | {
      status: "saved";
      draft_id: string;
      revision: number;
      committed_text: string;
      recovery_cleared: boolean;
    }
  | { status: "conflict"; current_text: string }
  | { status: "missing-target" }
  | { status: "stale-workspace" }
  | { status: "stale-generation" };

export type ResolveNoteDraftTargetResult =
  | {
      status: "live-coding";
      draft_id: string;
      interview_id: string;
      participant_label: string;
      committed_text: string;
    }
  | {
      status: "live-interview";
      draft_id: string;
      participant_label: string;
      committed_text: string;
    }
  | { status: "missing"; draft_id: string; reason: string };

export interface UpdateDepartureApproval {
  token: string;
  project_key: string | null;
  epoch: string | null;
  draft_write_seq: number;
}

export interface DepartureCompletion {
  intent_id: string;
  replayed: string;
}

export interface RecoveryStatus {
  available: boolean;
  error: string | null;
}

/** Native close/quit intent held for the frontend draft preflight. */
export interface NativeDepartureRequest {
  intent_id: string;
  kind: "close" | "quit";
}
