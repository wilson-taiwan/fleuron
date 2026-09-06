import { invoke } from "@tauri-apps/api/core";
import type { ExportConfig } from "./export-config";
import type {
  ActivityLogEntry,
  BackupInfo,
  AppPreferences,
  AppVersionInfo,
  RestoreOutcome,
  BeginNoteDraftInput,
  BeginNoteDraftResult,
  PutNoteDraftInput,
  PutNoteDraftResult,
  DiscardNoteDraftInput,
  DiscardNoteDraftResult,
  SaveNoteDraftInput,
  SaveNoteDraftResult,
  ResolveNoteDraftTargetResult,
  UpdateDepartureApproval,
  DepartureCompletion,
  RecoveryStatus,
  NoteDraftRecord,
  ClearWorkspaceInput,
  ClearWorkspaceResult,
  Code,
  CodedSegment,
  EnsureCodeAndApplyResult,
  MutateCodingEdgeResult,
  DeleteCodeMode,
  DeleteCodeResult,
  ExportResult,
  Interview,
  InterviewDeleteImpact,
  LiveWorkspaceSnapshot,
  ProjectFileEntry,
  ProjectInfo,
  RecentProject,
  CreatedGroup,
  GroupInfo,
  JoinedGroup,
  MembershipsCache,
  MembershipSummary,
  Redeemed,
  RenamedSelf,
  SegmentInput,
  SyncOutcome,
  SyncStatus,
  SyncV2Activation,
  SyncV2Readiness,
  SyncConflictDetail,
  UpdateCoordinatorStatus,
  TranscriptSegment,
  WorkspaceState,
  ProjectOpenSnapshot,
  JoinTargetVerdict,
  StudyLocation,
  RelinkOutcome,
  OpenMarkerStatus,
  LeftStudy,
  SegmentSpeakerChange,
} from "./types";
import type { StudyReadiness } from "./home-rows";

export const api = {
  createProject: (input: {
    parent_dir: string;
    project_name: string;
    title: string;
    coders: string[];
  }) => invoke<ProjectInfo>("create_project", { input }),

  openProject: (path: string) =>
    invoke<ProjectOpenSnapshot>("open_project", { path }),

  getLiveWorkspaceSnapshot: (activeInterviewId?: string | null) =>
    invoke<LiveWorkspaceSnapshot>("get_live_workspace_snapshot", {
      activeInterviewId: activeInterviewId ?? null,
    }),

  listSyncConflicts: () => invoke<SyncConflictDetail[]>("list_sync_conflicts"),

  closeProject: () => invoke<void>("close_project"),

  getProjectInfo: () => invoke<ProjectInfo>("get_project_info"),

  /**
   * Rewrite this folder's coder list and existing coding to the group name,
   * so local work and the roster stay one identity.
   */
  adoptProjectCoder: (from: string, to: string) =>
    invoke<ProjectInfo>("adopt_project_coder", { from, to }),

  /** Align this local project with the group title from the server. */
  adoptProjectTitle: (title: string) =>
    invoke<ProjectInfo>("adopt_project_title", { title }),

  listActivity: () => invoke<ActivityLogEntry[]>("list_activity"),

  listCodes: () => invoke<Code[]>("list_codes"),

  listRetiredCodes: () => invoke<Code[]>("list_retired_codes"),

  createCode: (input: {
    name: string;
    definition?: string;
    inclusion_criteria?: string;
    exclusion_criteria?: string;
    example?: string;
    parent_id?: string;
    color?: string;
  }) => invoke<Code>("create_code", { input }),

  ensureCodeAndApply: (input: {
    name: string;
    color?: string;
    interview_id: string;
    segment_id: string;
    coder_name: string;
    char_start?: number;
    char_end?: number;
  }) => invoke<EnsureCodeAndApplyResult>("ensure_code_and_apply", { input }),

  updateCode: (input: {
    id: string;
    name: string;
    definition?: string | null;
    inclusion_criteria?: string | null;
    exclusion_criteria?: string | null;
    example?: string | null;
    parent_id?: string | null;
    color: string;
  }) => invoke<Code>("update_code", { input }),

  deleteCode: (codeId: string, mode: DeleteCodeMode, coderName: string) =>
    invoke<DeleteCodeResult>("delete_code", { codeId, mode, coderName }),

  restoreCode: (codeId: string) => invoke<void>("restore_code", { codeId }),

  listInterviews: () => invoke<Interview[]>("list_interviews"),

  createInterview: (input: {
    participant_label: string;
    interview_date?: string;
    modality?: string;
    diagnosis_notes?: string;
    interviewers: string[];
  }) => invoke<Interview>("create_interview", { input }),

  updateInterview: (input: {
    id: string;
    participant_label: string;
    interview_date?: string | null;
    modality?: string | null;
    diagnosis_notes?: string | null;
  }) => invoke<Interview>("update_interview", { input }),

  interviewDeleteImpact: (interviewId: string) =>
    invoke<InterviewDeleteImpact>("interview_delete_impact", { interviewId }),

  deleteInterview: (interviewId: string, coderName: string) =>
    invoke<void>("delete_interview", { interviewId, coderName }),

  importSegments: (input: {
    interview_id: string;
    segments: SegmentInput[];
    raw_vtt_path?: string;
  }) => invoke<number>("import_segments", { input }),

  getSegments: (interviewId: string) =>
    invoke<TranscriptSegment[]>("get_segments", { interviewId }),

  setSegmentSpeaker: (
    segmentId: string,
    newSpeaker: string,
    includeFollowing: boolean,
  ) =>
    invoke<SegmentSpeakerChange[]>("set_segment_speaker", {
      segmentId,
      newSpeaker,
      includeFollowing,
    }),

  restoreSegmentSpeakers: (changes: SegmentSpeakerChange[]) =>
    invoke<void>("restore_segment_speakers", { changes }),

  setSegmentReviewed: (segmentId: string, reviewed: boolean) =>
    invoke<void>("set_segment_reviewed", { segmentId, reviewed }),

  listSegmentReviews: (interviewId: string) =>
    invoke<string[]>("list_segment_reviews", { interviewId }),

  applyCodes: (input: {
    interview_id: string;
    segment_id: string;
    code_ids: string[];
    coder_name: string;
    memo?: string;
    char_start?: number;
    char_end?: number;
  }) => invoke<CodedSegment>("apply_codes", { input }),

  mutateCodingEdge: (input: {
    interview_id: string;
    segment_id: string;
    code_id: string;
    coder_name: string;
    char_start?: number;
    char_end?: number;
    present: boolean;
  }) => invoke<MutateCodingEdgeResult>("mutate_coding_edge", { input }),

  patchCodingMemo: (input: { coded_segment_id: string; memo?: string }) =>
    invoke<CodedSegment>("patch_coding_memo", { input }),

  deleteCodedSegment: (codedSegmentId: string) =>
    invoke<void>("delete_coded_segment", { codedSegmentId }),

  listCodedSegments: (interviewId?: string, codeId?: string) =>
    invoke<CodedSegment[]>("list_coded_segments", {
      interviewId: interviewId ?? null,
      codeId: codeId ?? null,
    }),

  updateHubMemo: (interviewId: string, memo: string) =>
    invoke<void>("update_hub_memo", { interviewId, memo }),

  // ── Note drafts: local recovery + checked writes ──────────────────────────

  noteRecoveryStatus: () => invoke<RecoveryStatus>("note_recovery_status"),

  listNoteDrafts: () => invoke<NoteDraftRecord[]>("list_note_drafts"),

  beginNoteDraft: (input: BeginNoteDraftInput) =>
    invoke<BeginNoteDraftResult>("begin_note_draft", { input }),

  putNoteDraft: (input: PutNoteDraftInput) =>
    invoke<PutNoteDraftResult>("put_note_draft", { input }),

  discardNoteDraft: (input: DiscardNoteDraftInput) =>
    invoke<DiscardNoteDraftResult>("discard_note_draft", { input }),

  saveNoteDraft: (input: SaveNoteDraftInput) =>
    invoke<SaveNoteDraftResult>("save_note_draft", { input }),

  resolveNoteDraftTarget: (draftId: string) =>
    invoke<ResolveNoteDraftTargetResult>("resolve_note_draft_target", {
      draftId,
    }),

  /** Selftest only: redirect recovery storage at a temporary root. */
  setRecoveryRootForSelftest: (path: string | null) =>
    invoke<void>("set_recovery_root_for_selftest", { path }),

  approveUpdateDeparture: () =>
    invoke<UpdateDepartureApproval>("approve_update_departure"),

  completeNoteDeparture: (intentId: string, approved: boolean) =>
    invoke<DepartureCompletion>("complete_note_departure", {
      intentId,
      approved,
    }),

  clearWorkspace: (input: ClearWorkspaceInput) =>
    invoke<ClearWorkspaceResult>("clear_workspace", { input }),

  exportWithConfig: (
    targetDir: string,
    config: ExportConfig,
    reportHtml: string | null,
    frameworkMatrixCsv: string | null,
    coderName: string,
  ) =>
    invoke<ExportResult>("export_with_config", {
      targetDir,
      config,
      reportHtml,
      frameworkMatrixCsv,
      coderName,
    }),

  createBackup: (note?: string) =>
    invoke<BackupInfo>("create_backup", { note: note ?? null }),

  listBackups: () => invoke<BackupInfo[]>("list_backups"),

  /** Coding pulled from a colleague that is still waiting for its transcript. */
  pendingCodedCount: () => invoke<number>("pending_coded_count"),

  /**
   * Replace the project with a snapshot. The backend closes and reopens the
   * database around the swap, so **everything the store holds is stale when
   * this resolves** — callers must reload, not merge.
   */
  restoreBackup: (backupPath: string) =>
    invoke<RestoreOutcome>("restore_backup", { backupPath }),

  deleteBackup: (backupPath: string) =>
    invoke<void>("delete_backup", { backupPath }),

  inspectBackup: (backupPath: string) =>
    invoke<BackupInfo>("inspect_backup", { backupPath }),

  importBackup: (sourcePath: string) =>
    invoke<BackupInfo>("import_backup", { sourcePath }),

  /**
   * Claim any project double-clicked before this page could listen for it.
   *
   * Doubles as the signal that the frontend is now listening, so call it once,
   * after the `open-project-path` listener is registered — never before.
   */
  consumePendingOpen: () => invoke<string[]>("consume_pending_open"),

  getProjectsLibraryDir: () => invoke<string>("get_projects_library_dir"),

  /** Name of the sync service owning this path ("Box", "iCloud Drive"), or null. */
  cloudProviderForPath: (path: string) =>
    invoke<string | null>("cloud_provider_for_path", { path }),

  /** Set when the app's own default library is itself inside a synced folder. */
  librarySyncWarning: () => invoke<string | null>("library_sync_warning"),

  getWorkspaceState: () => invoke<WorkspaceState>("get_workspace_state"),

  saveWorkspaceState: (workspace: WorkspaceState) =>
    invoke<void>("save_workspace_state", { workspace }),

  readTextFile: (path: string) => invoke<string>("read_text_file", { path }),

  /** Reads a transcript in any supported container; unzips .docx on the way. */
  readTranscriptFile: (path: string) =>
    invoke<string>("read_transcript_file", { path }),

  /** Scans a folder for transcript files and returns their candidate contents. */
  scanTranscriptFolder: (folderPath: string) =>
    invoke<Array<{ path: string; name: string; raw_text: string }>>(
      "scan_transcript_folder",
      { folderPath },
    ),

  /** Computes the remote content hash for candidate segments without importing. */
  hashCandidateSegments: (segments: Array<{ index: number; text: string }>) =>
    invoke<string | null>("hash_candidate_segments", { segments }),

  listRecentProjects: () => invoke<RecentProject[]>("list_recent_projects"),

  recordRecentProject: (input: {
    path: string;
    title: string;
    last_handoff_by?: string | null;
    last_handoff_at?: string | null;
  }) => invoke<RecentProject[]>("record_recent_project", { input }),

  removeRecentProject: (path: string) =>
    invoke<RecentProject[]>("remove_recent_project", { path }),

  getAppPreferences: () => invoke<AppPreferences>("get_app_preferences"),

  setAppPreferences: (prefs: AppPreferences) =>
    invoke<AppPreferences>("set_app_preferences", { prefs }),

  getAppVersion: () => invoke<AppVersionInfo>("get_app_version"),

  listProjectFiles: () => invoke<ProjectFileEntry[]>("list_project_files"),

  copyProjectFile: (source: string, destination: string) =>
    invoke<void>("copy_project_file", { source, destination }),

  syncStatus: () => invoke<SyncStatus>("sync_status"),

  /**
   * Credentials go straight to the backend and are never held in component
   * state longer than the keystroke that produced them — the webview is the
   * one place in this app with a network stack pointed at the open internet.
   */
  syncSignIn: (email: string, password: string) =>
    invoke<void>("sync_sign_in", { email, password }),

  /** Bring back a sign-in remembered in the system keychain. */
  syncRestoreSession: () => invoke<boolean>("sync_restore_session"),

  syncSignOut: () => invoke<void>("sync_sign_out"),

  syncNow: () => invoke<SyncOutcome>("sync_now"),

  /** Deep verify local state against remote server records from epoch. Read-only. */
  syncDeepVerify: () => invoke<import("./types").DiagnosticsReport>("sync_deep_verify"),

  /** Reset cursors and run sync to repair. */
  syncRepair: () => invoke<SyncOutcome>("sync_repair"),

  /** Redacted plain text summary for clipboard copy. */
  syncDiagnosticsDump: () => invoke<string>("sync_diagnostics_dump"),

  /** Groups this account belongs to, with the name it files under in each. */
  listMemberships: () =>
    invoke<MembershipSummary[]>("sync_list_memberships"),

  /** Cached groups for offline / fast first-paint. */
  listCachedMemberships: () =>
    invoke<MembershipsCache>("list_cached_memberships"),

  /** Bind this local project to a shared one, and re-offer everything local. */
  syncJoinProject: (projectId: string) =>
    invoke<void>("sync_join_project", { projectId }),

  /**
   * Turn this project into a group, minting its key. The creator's membership
   * starts under their email address; follow with `syncSetMyCoderName`.
   */
  syncCreateProject: (title: string) =>
    invoke<CreatedGroup>("sync_create_project", { title }),

  /** Join a group by its key, choosing the name you file under. */
  syncJoinGroup: (key: string, coderName: string) =>
    invoke<JoinedGroup>("sync_join_group", { key, coderName }),

  /** The open project's group: key, roster, and per-person activity. */
  syncGroupInfo: () => invoke<GroupInfo>("sync_group_info"),

  syncV2Readiness: () => invoke<SyncV2Readiness>("sync_v2_readiness"),

  syncV2Activate: () => invoke<SyncV2Activation>("sync_v2_activate"),

  syncV2ResolveConflict: (
    conflictId: string,
    resolution: "keep_current" | "accept_proposal" | "custom",
    customValue?: string | number | boolean | null,
  ) => invoke<void>("sync_v2_resolve_conflict", {
    conflictId,
    resolution,
    customValue: customValue ?? null,
  }),

  /** Mint a fresh group key, retiring the old one. */
  syncResetGroupKey: () => invoke<string>("sync_reset_group_key"),

  /** Set a member's role (admin or coder). Admin-only. */
  syncSetMemberRole: (userId: string, role: string) =>
    invoke<void>("sync_set_member_role", { userId, role }),

  /** Remove a member from the group. Admin-only. */
  syncRemoveMember: (userId: string) =>
    invoke<void>("sync_remove_member", { userId }),

  /** Delete the entire group. Admin-only. Works with no project open. */
  syncDeleteGroup: (confirmTitle: string, projectId?: string) =>
    invoke<void>("sync_delete_group", { confirmTitle, projectId }),

  /** Leave the study. Removes caller's membership row while keeping their coding. */
  syncLeaveGroup: (projectId?: string) =>
    invoke<void>("sync_leave_group", { projectId }),

  /** Detach/unbind a study locally without touching the server. Works completely offline. */
  syncDetachLocal: (projectId?: string) =>
    invoke<void>("sync_detach_local", { projectId }),

  /** Returns count of interviews, coded segments, and memos for a project folder. */
  projectDeletionSummary: (path: string) =>
    invoke<{
      interview_count: number;
      coded_segment_count: number;
      memo_count: number;
    }>("project_deletion_summary", { path }),

  /** Moves a project folder to Trash. */
  deleteProjectFolder: (path: string) =>
    invoke<void>("delete_project_folder", { path }),

  /**
   * Change the name you file under in this group. The server rewrites your
   * coded rows to the new name in the same transaction.
   */
  syncSetMyCoderName: (coderName: string) =>
    invoke<RenamedSelf>("sync_set_my_coder_name", { coderName }),

  /**
   * Create an account. Resolves false when the server wants an email
   * confirmation first, so the account exists but cannot sign in yet.
   */
  syncSignUp: (email: string, password: string) =>
    invoke<boolean>("sync_sign_up", { email, password }),

  /** Email a reset code. Does not reveal whether that inbox has an account. */
  syncRequestPasswordReset: (email: string) =>
    invoke<void>("sync_request_password_reset", { email }),

  /**
   * Set a new password from the email's code, and sign in.
   * Pass whichever secret parseRecoveryPaste found; unused fields stay null.
   */
  syncCompletePasswordReset: (input: {
    email: string;
    password: string;
    token?: string | null;
    tokenHash?: string | null;
    accessToken?: string | null;
    refreshToken?: string | null;
  }) =>
    invoke<void>("sync_complete_password_reset", {
      email: input.email,
      password: input.password,
      token: input.token ?? null,
      tokenHash: input.tokenHash ?? null,
      accessToken: input.accessToken ?? null,
      refreshToken: input.refreshToken ?? null,
    }),

  /**
   * Redeem a legacy single-use invitation. Group keys superseded these; this
   * stays so a code already in someone's message still works.
   */
  syncRedeemInvite: (code: string) =>
    invoke<Redeemed>("sync_redeem_invite", { code }),

  /** Render HTML report to PDF file via native platform renderer. */
  renderReportPdf: (html: string, outPath: string) =>
    invoke<void>("render_report_pdf", { html, outPath }),

  getUpdateStatus: () => invoke<UpdateCoordinatorStatus>("get_update_status"),

  updateCheck: () => invoke<UpdateCoordinatorStatus>("update_check"),

  updateDownload: () => invoke<UpdateCoordinatorStatus>("update_download"),

  updateCancelDownload: () => invoke<UpdateCoordinatorStatus>("update_cancel_download"),

  updateInstall: (approval?: string | null) =>
    invoke<UpdateCoordinatorStatus>("update_install", {
      approval: approval ?? null,
    }),

  /** Refresh cached server schema version. */
  syncRefreshServerSchema: () =>
    invoke<number | null>("sync_refresh_server_schema"),

  /** Reconcile any in-flight unbind recorded in the intent journal. */
  syncReconcilePendingUnbind: () =>
    invoke<void>("sync_reconcile_pending_unbind"),

  /** Generate redacted, in-app diagnostic report. */
  generateDiagnosticReport: () => invoke<string>("generate_diagnostic_report"),

  /** Read active crash log text. */
  readCrashLog: () => invoke<string>("read_crash_log"),

  /** Check if last exit was unclean and clear the flag. */
  takeUncleanExitNotice: () => invoke<boolean>("take_unclean_exit_notice"),

  inspectJoinTarget: (parentDir: string, slug: string, projectId: string) =>
    invoke<JoinTargetVerdict>("inspect_join_target", {
      parentDir,
      slug,
      projectId,
    }),

  listLeftStudies: () => invoke<LeftStudy[]>("list_left_studies"),

  resolveStudyLocation: (path: string) =>
    invoke<StudyLocation>("resolve_study_location", { path }),

  autoRelinkStudy: (path: string, expectedProjectId?: string) =>
    invoke<RelinkOutcome>("auto_relink_study", {
      path,
      expectedProjectId: expectedProjectId ?? null,
    }),

  studyReadiness: (path: string) =>
    invoke<StudyReadiness>("study_readiness", { path }),

  checkOpenMarker: (path: string) =>
    invoke<OpenMarkerStatus>("check_open_marker", { path }),

  heartbeatOpenMarker: () => invoke<void>("heartbeat_open_marker"),
};
