/**
 * Context-aware StudyProblem contract for study lifecycle flows.
 * Resolves safe messages, allowed actions, retryability, and sanitized diagnostics.
 */

import { fileManagerName } from "./platform";

export type StudyLifecycleOperation =
  | "open"
  | "create"
  | "join"
  | "setup"
  | "leave"
  | "delete"
  | "detach"
  | "restore"
  | "inspect";

export type StudyLifecycleStage =
  | "checking"
  | "preflight"
  | "folder_allocated"
  | "created"
  | "template_seeded"
  | "transcripts_linked"
  | "preserved"
  | "remote_confirmed"
  | "local_unbound"
  | "handles_closed"
  | "folder_trashed"
  | "home_reconciled"
  | "complete";

export type StudyProblemCode =
  | "existing_verified_folder"
  | "matching_detached_folder"
  | "unrelated_name_collision"
  | "multiple_copies"
  | "missing_folder"
  | "drive_disconnected"
  | "cloud_placeholder"
  | "cloud_provider_unavailable"
  | "permission_denied"
  | "read_only_location"
  | "disk_full"
  | "file_locked"
  | "invalid_project"
  | "newer_schema"
  | "unknown_inspection_error"
  | "create_complete_template_failed"
  | "create_complete_import_failed"
  | "joined_setup_incomplete"
  | "missing_transcripts"
  | "hash_mismatch"
  | "invalid_expired_group_key"
  | "duplicate_coder_name"
  | "expired_session"
  | "offline_before_server_removal"
  | "missing_lifecycle_service_capability"
  | "last_member"
  | "sole_admin_with_others"
  | "unsynced_changes"
  | "unfinished_private_notes"
  | "summary_unreadable"
  | "server_denied_removal"
  | "response_lost"
  | "leave_complete_trash_failed"
  | "group_deleted_trash_failed"
  | "server_complete_home_cache_failed"
  | "local_trash_failed"
  | "membership_removed_elsewhere"
  | "group_deleted_elsewhere"
  | "unexpected_failure";

export type StudyActionId =
  | "open_study"
  | "show_folder"
  | "reconnect_study"
  | "choose_location"
  | "select_path"
  | "locate_folder"
  | "restore_backup"
  | "remove_recent"
  | "retry"
  | "retry_inspection"
  | "retry_template"
  | "open_without_template"
  | "retry_import"
  | "choose_another_file"
  | "finish_later"
  | "continue_setup"
  | "link_transcripts"
  | "edit_key"
  | "focus_coder_name"
  | "sign_in"
  | "continue_locally"
  | "stop_syncing_locally"
  | "save_local_copy"
  | "manage_members"
  | "sync_now"
  | "explicit_discard"
  | "refresh_permissions"
  | "check_status"
  | "return_home"
  | "keep_local_study"
  | "refresh_list"
  | "copy_diagnostic"
  | "check_for_updates"
  | "cancel";

export interface StudyProblem {
  code: StudyProblemCode;
  operation: StudyLifecycleOperation;
  target?: string;
  completedStage?: StudyLifecycleStage;
  message: string;
  detail?: string;
  diagnosticId?: string;
  nextActions: StudyActionId[];
  isRetryable: boolean;
}

/** Scrub sensitive tokens, passwords, raw SQL errors, and credentials. */
export function scrubDiagnostics(raw: string): string {
  return raw
    .replace(/[a-zA-Z0-9_-]{20,}\.[a-zA-Z0-9_-]{20,}\.[a-zA-Z0-9_-]{20,}/g, "[REDACTED_JWT]")
    .replace(/(bearer\s+)[a-zA-Z0-9_\-\.]+/gi, "$1[REDACTED]")
    .replace(/(password|token|secret|key)=([^&\s]+)/gi, "$1=[REDACTED]")
    .replace(/supabase_key=[^&\s]+/gi, "supabase_key=[REDACTED]")
    .replace(/PGRST\d+/g, "SERVER_ERROR")
    .replace(/relation "[^"]+" does not exist/gi, "database entity error")
    .trim();
}

/** Action button label mappings. */
export const ACTION_LABELS: Record<StudyActionId, string> = {
  open_study: "Open study",
  show_folder: "Show Folder",
  reconnect_study: "Reconnect this study",
  choose_location: "Choose location",
  select_path: "Select a path",
  locate_folder: "Locate folder",
  restore_backup: "Restore a study backup",
  remove_recent: "Remove from recent list",
  retry: "Retry",
  retry_inspection: "Retry inspection",
  retry_template: "Retry starter codes",
  open_without_template: "Open without starter codes",
  retry_import: "Retry import",
  choose_another_file: "Choose another file",
  finish_later: "Finish later",
  continue_setup: "Continue setup",
  link_transcripts: "Link transcripts",
  edit_key: "Edit key",
  focus_coder_name: "Change coding name",
  sign_in: "Sign in",
  continue_locally: "Continue locally",
  stop_syncing_locally: "Stop syncing locally",
  save_local_copy: "Save a local copy",
  manage_members: "Manage members",
  sync_now: "Sync now",
  explicit_discard: "Discard local changes",
  refresh_permissions: "Refresh permissions",
  check_status: "Check status",
  return_home: "Return home",
  keep_local_study: "Keep local study",
  refresh_list: "Refresh list",
  copy_diagnostic: "Copy diagnostic summary",
  check_for_updates: "Check for updates",
  cancel: "Cancel",
};

/** Classify error into typed StudyProblem according to required matrix. */
export function classifyStudyProblem(
  err: unknown,
  operation: StudyLifecycleOperation,
  context?: {
    target?: string;
    completedStage?: StudyLifecycleStage;
    studyTitle?: string;
    hasLocalCopy?: boolean;
    isAdmin?: boolean;
    memberCount?: number;
    successorName?: string;
  }
): StudyProblem {
  const rawMsg = err instanceof Error ? err.message : String(err || "");
  const scrubbed = scrubDiagnostics(rawMsg);
  const diagnosticId = `err-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  const title = context?.studyTitle ? ` “${context.studyTitle}”` : "";

  // 1. Two folders one group
  if (rawMsg.includes("TWO_FOLDERS_ONE_GROUP")) {
    return {
      code: "multiple_copies",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "More than one local copy matches this study. Choose which one to use.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["select_path", "show_folder", "cancel"],
      isRetryable: false,
    };
  }

  // 2. Permission denied
  if (rawMsg.includes("permission_denied") || rawMsg.toLowerCase().includes("permission denied")) {
    return {
      code: "permission_denied",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "Fleuron cannot access this folder. Choose another location or grant access, then retry.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["choose_location", "retry", "cancel"],
      isRetryable: true,
    };
  }

  // 3. Storage full / Disk full
  if (rawMsg.includes("storage_full") || rawMsg.toLowerCase().includes("space")) {
    return {
      code: "disk_full",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "There is not enough space to finish this step. Free space, then retry.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["retry", "choose_location", "cancel"],
      isRetryable: true,
    };
  }

  // 4. File locked
  if (rawMsg.includes("file_in_use") || rawMsg.toLowerCase().includes("locked") || rawMsg.toLowerCase().includes("sharing violation")) {
    return {
      code: "file_locked",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "Another process is using the study folder. Close it there, then retry.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["retry", "show_folder", "cancel"],
      isRetryable: true,
    };
  }

  // 5. Cloud placeholder
  if (rawMsg.includes("content_not_downloaded")) {
    return {
      code: "cloud_placeholder",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: `Download this study's files in ${fileManagerName}, then retry.`,
      detail: scrubbed,
      diagnosticId,
      nextActions: ["show_folder", "retry", "cancel"],
      isRetryable: true,
    };
  }

  // 6. Missing folder
  if (rawMsg.includes("path_unavailable") || rawMsg.toLowerCase().includes("no such file") || rawMsg.toLowerCase().includes("not found")) {
    return {
      code: "missing_folder",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: `Fleuron cannot find this study${title} at its saved location.`,
      detail: scrubbed,
      diagnosticId,
      nextActions: ["locate_folder", "restore_backup", "remove_recent", "cancel"],
      isRetryable: false,
    };
  }

  // 7. Invalid project
  if (rawMsg.includes("invalid_project") || rawMsg.toLowerCase().includes("not a valid fleuron")) {
    return {
      code: "invalid_project",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "This folder does not contain a readable Fleuron study.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["choose_location", "restore_backup", "cancel"],
      isRetryable: false,
    };
  }

  // 8. Newer schema
  if (rawMsg.toLowerCase().includes("newer schema") || rawMsg.toLowerCase().includes("unsupported user_version")) {
    return {
      code: "newer_schema",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "This study needs a newer version of Fleuron.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["check_for_updates", "cancel"],
      isRetryable: false,
    };
  }

  // 9. Last member
  if (rawMsg.toLowerCase().includes("last member") || rawMsg.toLowerCase().includes("only member")) {
    return {
      code: "last_member",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "You are the only member. Keep a local study, add another member, or delete the shared study.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["stop_syncing_locally", "save_local_copy", "manage_members", "cancel"],
      isRetryable: false,
    };
  }

  // 10. Trash failed
  if (rawMsg.toLowerCase().includes("trash") && rawMsg.toLowerCase().includes("failed")) {
    return {
      code: "local_trash_failed",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "The study could not be moved to Trash/Recycle Bin. The folder has been kept.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["retry", "show_folder", "cancel"],
      isRetryable: true,
    };
  }

  // 11. Response lost / unknown remote outcome
  if (rawMsg.toLowerCase().includes("timeout") || rawMsg.toLowerCase().includes("network error") || rawMsg.toLowerCase().includes("fetch failed")) {
    return {
      code: "response_lost",
      operation,
      target: context?.target,
      completedStage: context?.completedStage,
      message: "Fleuron is checking whether the group change completed. Your local files have been kept.",
      detail: scrubbed,
      diagnosticId,
      nextActions: ["check_status", "return_home"],
      isRetryable: false,
    };
  }

  // Default fallback
  const completedText = context?.completedStage
    ? ` Stage reached: ${context.completedStage}.`
    : "";
  return {
    code: "unexpected_failure",
    operation,
    target: context?.target,
    completedStage: context?.completedStage,
    message: `Fleuron could not finish ${operation}.${completedText} Your files have not been damaged.`,
    detail: scrubbed,
    diagnosticId,
    nextActions: ["retry", "return_home", "cancel"],
    isRetryable: true,
  };
}
