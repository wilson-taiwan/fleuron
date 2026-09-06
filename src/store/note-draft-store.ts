import { create } from "zustand";
import { api } from "../lib/api";
import type {
  NoteDraftKind,
  NoteDraftRecord,
  ResolveNoteDraftTargetResult,
} from "../lib/types";

export type DraftKind = NoteDraftKind;
export type DraftSaveState = "clean" | "dirty" | "saving" | "error" | "conflict";
export type DraftRecoveryState =
  | "idle"
  | "backing-up"
  | "backed-up"
  | "unavailable"
  | "error";

export interface DraftTarget {
  kind: DraftKind;
  /** Coding id, or interview id for interview notes. */
  targetId: string;
  interviewId: string;
  coderName?: string | null;
  participantLabel: string;
  segmentId?: string | null;
  segmentIndex?: number | null;
  charStart?: number | null;
  charEnd?: number | null;
  quoteText?: string | null;
}

export interface NoteDraftEntry {
  key: string;
  projectKey: string;
  kind: DraftKind;
  targetId: string;
  interviewId: string;
  epoch: string;
  draftId: string;
  /** Latest revision in memory (increments on every actual text change). */
  revision: number;
  /** Latest revision acknowledged by the recovery store. */
  ackedRevision: number;
  /** Last acknowledged committed text. Dirty is derived: draftText !== baseText. */
  baseText: string;
  draftText: string;
  saveState: DraftSaveState;
  saveError: string | null;
  /** Live committed text when a save met a changed note. */
  conflictText: string | null;
  targetMissing: boolean;
  epochStale: boolean;
  recoveryState: DraftRecoveryState;
  recoveryError: string | null;
  /** A put is in flight; the latest text waits to follow it. */
  putInFlight: boolean;
  queuedText: string | null;
  /** A commit is in flight; a trailing save was requested meanwhile. */
  saveInFlight: boolean;
  saveQueued: boolean;
  /** Explicit discard landed while writes were queued: drop them silently. */
  discardRequested: boolean;
  /** Captured source context, reused if the generation must be reborn. */
  context: {
    coderName: string | null;
    participantLabel: string;
    segmentId: string | null;
    segmentIndex: number | null;
    charStart: number | null;
    charEnd: number | null;
    quoteText: string | null;
  };
  updatedAt: number;
}

export function draftKey(projectKey: string, kind: string, targetId: string): string {
  return `${projectKey}::${kind}::${targetId}`;
}

export function isDraftDirty(entry: Pick<NoteDraftEntry, "draftText" | "baseText">): boolean {
  return entry.draftText !== entry.baseText;
}

interface NoteDraftStore {
  workspace: { projectKey: string; epoch: string } | null;
  entries: Record<string, NoteDraftEntry>;
  /** The one coding with an open inline editor (many drafts, one editor). */
  activeInlineCodingId: string | null;
  recoveryAvailable: boolean;
  recoveryError: string | null;

  bindWorkspace: (projectKey: string, epoch: string) => void;
  clearWorkspace: () => void;
  setActiveInline: (codingId: string | null) => void;

  getEntry: (projectKey: string, kind: string, targetId: string) => NoteDraftEntry | null;
  getEntryByKey: (key: string) => NoteDraftEntry | null;
  dirtyEntries: (projectKey?: string) => NoteDraftEntry[];
  hasDirty: (projectKey?: string) => boolean;

  /** Start or resume editing for a live target (serialized per target). */
  beginDraft: (target: DraftTarget) => Promise<NoteDraftEntry>;
  /** Record a keystroke: memory first, recovery write serialized behind it. */
  editDraft: (key: string, text: string) => void;
  /** Commit through CAS. Closes nothing; callers decide on ack. */
  saveDraft: (key: string) => Promise<boolean>;
  /** Conditional discard of the live generation. */
  discardDraft: (key: string) => Promise<boolean>;
  retryRecovery: (key: string) => void;
  /** Conflict modal choices. */
  saveMineOver: (key: string) => Promise<boolean>;
  useSavedVersion: (key: string) => Promise<boolean>;
  keepEditing: (key: string) => void;
  /** Adopt committed content from a snapshot (clean only; dirty → conflict). */
  adoptSnapshotText: (kind: string, targetId: string, committedText: string) => void;
  /** After a workspace rebind, re-begin entries stranded on an old epoch. */
  rebaseStaleEntries: () => Promise<void>;
  /** Await every in-flight put/save (departure/export preflight step one). */
  flushPending: () => Promise<void>;
  refreshRecoveryStatus: () => Promise<void>;
  removeEntry: (key: string) => void;
  /** Temporary edit freeze during departure/export preflight and update install. */
  frozen: boolean;
  setFrozen: (frozen: boolean) => void;
}

function friendlyRecoveryError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  // The backend already words recovery failures for display; pass through
  // anything that names recovery, and wrap raw transport failures.
  if (/recovery|unavailable|backup/i.test(message)) return message;
  return `Draft backup failed (${message}). Your text is still here — retry to back it up.`;
}

function friendlySaveError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (/STALE_WORKSPACE/i.test(message)) {
    return "The study changed under this note. Review it and save again.";
  }
  return message;
}

export const useNoteDraftStore = create<NoteDraftStore>()((set, get) => {
  function patchEntry(key: string, patch: Partial<NoteDraftEntry>) {
    set((state) => {
      const entry = state.entries[key];
      if (!entry) return state;
      return {
        entries: { ...state.entries, [key]: { ...entry, ...patch, updatedAt: Date.now() } },
      };
    });
  }

  /** Serialize one recovery put for an entry, coalescing to the latest text. */
  async function runPut(key: string): Promise<void> {
    const entry = get().entries[key];
    if (!entry || entry.discardRequested) return;
    if (entry.putInFlight) return;
    const workspace = get().workspace;
    if (!workspace) {
      patchEntry(key, { recoveryState: "unavailable", recoveryError: "No study is open." });
      return;
    }
    patchEntry(key, { putInFlight: true, recoveryState: "backing-up", recoveryError: null });
    try {
      // Capture the latest queued text; the loop replays until memory and
      // the acknowledgement agree or the backend reports a verdict.
      for (;;) {
        const current = get().entries[key];
        if (!current || current.discardRequested) break;
        const text = current.queuedText ?? current.draftText;
        const expected = current.ackedRevision;
        const result = await api.putNoteDraft({
          project_key: current.projectKey,
          epoch: workspace.epoch,
          draft_id: current.draftId,
          expected_revision: expected,
          draft_text: text,
        });
        const latest = get().entries[key];
        if (!latest || latest.discardRequested) break;
        if (result.status === "stored") {
          const acked = result.record.revision;
          const stillQueued =
            latest.queuedText !== null && latest.queuedText !== result.record.draft_text;
          patchEntry(key, {
            ackedRevision: acked,
            queuedText: stillQueued ? latest.queuedText : null,
            recoveryState: stillQueued ? "backing-up" : "backed-up",
          });
          if (!stillQueued) break;
          continue;
        }
        if (result.status === "revision-mismatch") {
          // Another writer (or our own retried save cleanup) moved the
          // stored revision: adopt the winner and replay the latest text.
          patchEntry(key, { ackedRevision: result.record.revision });
          continue;
        }
        if (result.status === "stale-generation") {
          // Our generation died (save cleanup or explicit discard raced
          // us). An explicit discard owns that outcome — drop the queue.
          // Otherwise re-begin a fresh generation and replay the text.
          if (get().entries[key]?.discardRequested) break;
          const rebased = await rebeginForLatest(key);
          if (!rebased) {
            patchEntry(key, {
              recoveryState: "error",
              recoveryError: "This note was discarded elsewhere. Reopen it to keep editing.",
            });
            break;
          }
          continue;
        }
        // Stale workspace: stop the loop and flag for rebase; the text
        // stays in memory and the entry is reported by flushPending.
        patchEntry(key, { epochStale: true, recoveryState: "error", recoveryError: friendlySaveError("STALE_WORKSPACE") });
        break;
      }
    } catch (error) {
      const live = get().entries[key];
      if (live && !live.discardRequested) {
        patchEntry(key, { recoveryState: "error", recoveryError: friendlyRecoveryError(error) });
      }
    } finally {
      const live = get().entries[key];
      if (live) patchEntry(key, { putInFlight: false });
    }
  }

  /** Re-begin after our generation died mid-queue; replays nothing itself. */
  async function rebeginForLatest(key: string): Promise<boolean> {
    const entry = get().entries[key];
    const workspace = get().workspace;
    if (!entry || !workspace) return false;
    try {
      const begun = await api.beginNoteDraft({
        project_key: entry.projectKey,
        epoch: workspace.epoch,
        kind: entry.kind,
        target_id: entry.targetId,
        interview_id: entry.interviewId,
        coder_name: entry.context.coderName,
        participant_label: entry.context.participantLabel,
        segment_id: entry.context.segmentId,
        segment_index: entry.context.segmentIndex,
        char_start: entry.context.charStart,
        char_end: entry.context.charEnd,
        quote_text: entry.context.quoteText,
      });
      if (begun.status !== "active") return false;
      patchEntry(key, {
        draftId: begun.record.draft_id,
        ackedRevision: begun.record.revision,
        baseText: begun.committed_text,
        epochStale: false,
        recoveryState: "backing-up",
        recoveryError: null,
      });
      return true;
    } catch {
      return false;
    }
  }

  async function runSave(key: string): Promise<boolean> {
    const entry = get().entries[key];
    const workspace = get().workspace;
    if (!entry || !workspace) return false;
    if (entry.saveInFlight) {
      patchEntry(key, { saveQueued: true });
      return false;
    }
    if (!isDraftDirty(entry) && entry.saveState !== "error") return true;
    patchEntry(key, {
      saveInFlight: true,
      saveQueued: false,
      saveState: "saving",
      saveError: null,
    });
    const capturedRevision = entry.revision;
    const capturedText = entry.draftText;
    const capturedBase = entry.baseText;
    const capturedDraftId = entry.draftId;
    try {
      const recoveryOk = get().recoveryAvailable;
      const result = await api.saveNoteDraft({
        project_key: entry.projectKey,
        epoch: workspace.epoch,
        // Memory-only path when recovery is down: the backend runs the same
        // CAS checks and the recovery warning travels separately.
        draft_id: recoveryOk ? capturedDraftId : "",
        revision: recoveryOk ? capturedRevision : entry.ackedRevision,
        kind: entry.kind,
        target_id: entry.targetId,
        expected_saved_text: capturedBase,
        draft_text: capturedText,
      });
      if (result.status === "saved") {
        const live = get().entries[key];
        const typedMeanwhile = live ? live.revision !== capturedRevision : false;
        patchEntry(key, {
          baseText: result.committed_text,
          saveState: typedMeanwhile ? "dirty" : "clean",
          saveError: null,
          conflictText: null,
          targetMissing: false,
          epochStale: false,
          saveInFlight: false,
        });
        if (!recoveryOk) {
          patchEntry(key, {
            recoveryState: "unavailable",
            recoveryError:
              "Saved, but the local backup is still unavailable. Retry the backup from Unfinished notes.",
          });
        } else if (!result.recovery_cleared && live && live.revision !== capturedRevision) {
          // Newer edits survived cleanup by design; make sure their backup
          // follows them.
          void runPut(key);
        }
        const after = get().entries[key];
        if (after && after.saveQueued && isDraftDirty(after)) {
          patchEntry(key, { saveQueued: false });
          return runSave(key);
        }
        if (after) patchEntry(key, { saveQueued: false });
        return true;
      }
      if (result.status === "conflict") {
        patchEntry(key, {
          saveState: "conflict",
          conflictText: result.current_text,
          saveError: null,
          saveInFlight: false,
          saveQueued: false,
        });
        return false;
      }
      if (result.status === "missing-target") {
        patchEntry(key, {
          saveState: "error",
          saveError: "The original coding is no longer available. Your text is kept in Unfinished notes.",
          targetMissing: true,
          saveInFlight: false,
          saveQueued: false,
        });
        return false;
      }
      if (result.status === "stale-generation") {
        patchEntry(key, {
          saveState: "error",
          saveError: "This note was discarded. Reopen it to start a fresh note.",
          saveInFlight: false,
          saveQueued: false,
        });
        return false;
      }
      // Stale workspace.
      patchEntry(key, {
        saveState: "error",
        saveError: friendlySaveError("STALE_WORKSPACE"),
        epochStale: true,
        saveInFlight: false,
        saveQueued: false,
      });
      return false;
    } catch (error) {
      patchEntry(key, {
        saveState: "error",
        saveError: `Could not save note: ${friendlySaveError(error)}`,
        saveInFlight: false,
      });
      return false;
    }
  }

  return {
    workspace: null,
    entries: {},
    activeInlineCodingId: null,
    recoveryAvailable: true,
    recoveryError: null,
    frozen: false,

    setFrozen: (frozen) => set({ frozen }),

    bindWorkspace: (projectKey, epoch) => {
      const previous = get().workspace;
      set({ workspace: { projectKey, epoch } });
      if (
        previous &&
        (previous.projectKey !== projectKey || previous.epoch !== epoch)
      ) {
        void get().rebaseStaleEntries();
      }
      void get().refreshRecoveryStatus();
    },

    clearWorkspace: () =>
      set({ workspace: null, entries: {}, activeInlineCodingId: null }),

    setActiveInline: (codingId) => set({ activeInlineCodingId: codingId }),

    getEntry: (projectKey, kind, targetId) =>
      get().entries[draftKey(projectKey, kind, targetId)] ?? null,

    getEntryByKey: (key) => get().entries[key] ?? null,

    dirtyEntries: (projectKey) =>
      Object.values(get().entries).filter(
        (entry) =>
          isDraftDirty(entry) &&
          (projectKey === undefined || entry.projectKey === projectKey),
      ),

    hasDirty: (projectKey) => get().dirtyEntries(projectKey).length > 0,

    beginDraft: async (target) => {
      const workspace = get().workspace;
      if (!workspace) throw new Error("No study is open.");
      const key = draftKey(workspace.projectKey, target.kind, target.targetId);
      const seen = get().entries[key];
      // Same target, live generation: focus the existing draft, never reseed.
      if (seen && !seen.epochStale) return seen;
      let begun;
      try {
        begun = await api.beginNoteDraft({
          project_key: workspace.projectKey,
          epoch: workspace.epoch,
          kind: target.kind,
          target_id: target.targetId,
          interview_id: target.interviewId,
          coder_name: target.coderName ?? null,
          participant_label: target.participantLabel,
          segment_id: target.segmentId ?? null,
          segment_index: target.segmentIndex ?? null,
          char_start: target.charStart ?? null,
          char_end: target.charEnd ?? null,
          quote_text: target.quoteText ?? null,
        });
      } catch (error) {
        throw new Error(friendlyRecoveryError(error));
      }
      if (begun.status === "stale-workspace") {
        throw new Error(friendlySaveError("STALE_WORKSPACE"));
      }
      if (begun.status === "missing-target") {
        throw new Error("The original coding is no longer available.");
      }
      const record: NoteDraftRecord = begun.record;
      const entry: NoteDraftEntry = {
        key,
        projectKey: workspace.projectKey,
        kind: target.kind,
        targetId: target.targetId,
        interviewId: target.interviewId,
        epoch: workspace.epoch,
        draftId: record.draft_id,
        revision: record.revision,
        ackedRevision: record.revision,
        baseText: begun.committed_text,
        // Resume the acknowledged draft text, not the committed text: the
        // whole point of reopening is getting the unfinished words back.
        draftText: record.draft_text,
        saveState: record.draft_text !== begun.committed_text ? "dirty" : "clean",
        saveError: null,
        conflictText: null,
        targetMissing: false,
        epochStale: false,
        recoveryState: "backed-up",
        recoveryError: null,
        putInFlight: false,
        queuedText: null,
        saveInFlight: false,
        saveQueued: false,
        discardRequested: false,
        context: {
          coderName: target.coderName ?? null,
          participantLabel: target.participantLabel,
          segmentId: target.segmentId ?? null,
          segmentIndex: target.segmentIndex ?? null,
          charStart: target.charStart ?? null,
          charEnd: target.charEnd ?? null,
          quoteText: target.quoteText ?? null,
        },
        updatedAt: Date.now(),
      };
      set((state) => ({ entries: { ...state.entries, [key]: entry } }));
      // If the committed text moved under a retained draft, surface the
      // comparison instead of silently adopting either side.
      if (record.base_text !== begun.committed_text && record.draft_text !== begun.committed_text) {
        patchEntry(key, { saveState: "conflict", conflictText: begun.committed_text });
      }
      return get().entries[key]!;
    },

    editDraft: (key, text) => {
      const entry = get().entries[key];
      if (!entry || entry.draftText === text) return;
      const revision = entry.revision + 1;
      const dirty = text !== entry.baseText;
      patchEntry(key, {
        draftText: text,
        revision,
        queuedText: text,
        saveState: entry.saveState === "conflict" ? "conflict" : dirty ? "dirty" : "clean",
        recoveryState:
          get().recoveryAvailable || entry.putInFlight ? "backing-up" : "unavailable",
      });
      if (!get().recoveryAvailable) {
        patchEntry(key, {
          recoveryState: "unavailable",
          recoveryError:
            "Draft recovery unavailable — your text is held in memory only. Retry the backup before leaving.",
        });
        return;
      }
      void runPut(key);
    },

    saveDraft: (key) => runSave(key),

    discardDraft: async (key) => {
      const entry = get().entries[key];
      if (!entry) return true;
      // Cancel the old edit queue first: an explicit discard owns the
      // outcome, and no queued put may resurrect the text afterwards.
      patchEntry(key, { discardRequested: true, queuedText: null });
      try {
        await api.discardNoteDraft({ draft_id: entry.draftId });
      } catch (error) {
        patchEntry(key, { discardRequested: false });
        patchEntry(key, {
          saveState: "error",
          saveError: `Could not discard the draft: ${friendlySaveError(error)} Nothing was deleted.`,
        });
        return false;
      }
      // Restore the committed text locally and drop the entry: the draft is
      // gone from recovery, so there is nothing left to own.
      set((state) => {
        const next = { ...state.entries };
        delete next[key];
        return { entries: next };
      });
      return true;
    },

    retryRecovery: (key) => {
      const entry = get().entries[key];
      if (!entry) return;
      patchEntry(key, { discardRequested: false, recoveryError: null });
      void get()
        .refreshRecoveryStatus()
        .then(() => {
          const live = get().entries[key];
          if (!live) return;
          if (isDraftDirty(live)) {
            patchEntry(key, { queuedText: live.draftText });
            void runPut(key);
          } else {
            patchEntry(key, { recoveryState: "backed-up" });
          }
        });
    },

    saveMineOver: async (key) => {
      const entry = get().entries[key];
      if (!entry || entry.saveState !== "conflict" || entry.conflictText === null) {
        return false;
      }
      // Re-CAS against the version shown: if the note moved again, the
      // comparison refreshes and asks once more instead of overwriting.
      patchEntry(key, {
        baseText: entry.conflictText,
        saveState: "dirty",
        conflictText: null,
      });
      return runSave(key);
    },

    useSavedVersion: async (key) => {
      const entry = get().entries[key];
      if (!entry || entry.conflictText === null) return false;
      const saved = entry.conflictText;
      patchEntry(key, {
        draftText: saved,
        baseText: saved,
        queuedText: saved,
        saveState: "clean",
        saveError: null,
        conflictText: null,
      });
      // Explicitly drop the unfinished draft from recovery after the local
      // state already reflects the saved version.
      try {
        await api.discardNoteDraft({ draft_id: entry.draftId });
      } catch {
        // The text the user chose is already on screen; a leftover recovery
        // row is a wart, not data loss. Flag it for the next retry.
        patchEntry(key, {
          recoveryState: "error",
          recoveryError: "The saved version is showing, but the old backup was not removed. Retry to clean it up.",
        });
        return true;
      }
      set((state) => {
        const next = { ...state.entries };
        delete next[key];
        return { entries: next };
      });
      return true;
    },

    keepEditing: (key) => {
      patchEntry(key, { saveState: "dirty" });
    },

    adoptSnapshotText: (kind, targetId, committedText) => {
      for (const entry of Object.values(get().entries)) {
        if (entry.kind !== kind || entry.targetId !== targetId) continue;
        if (!isDraftDirty(entry)) {
          patchEntry(entry.key, { baseText: committedText, draftText: committedText });
        } else if (entry.baseText !== committedText) {
          patchEntry(entry.key, {
            saveState: "conflict",
            conflictText: committedText,
          });
        }
      }
    },

    rebaseStaleEntries: async () => {
      const workspace = get().workspace;
      if (!workspace) return;
      for (const entry of Object.values(get().entries)) {
        if (!entry.epochStale && entry.epoch === workspace.epoch) continue;
        try {
          const begun = await api.beginNoteDraft({
            project_key: workspace.projectKey,
            epoch: workspace.epoch,
            kind: entry.kind,
            target_id: entry.targetId,
            interview_id: entry.interviewId,
            coder_name: entry.context.coderName,
            participant_label: entry.context.participantLabel,
            segment_id: entry.context.segmentId,
            segment_index: entry.context.segmentIndex,
            char_start: entry.context.charStart,
            char_end: entry.context.charEnd,
            quote_text: entry.context.quoteText,
          });
          if (begun.status !== "active") {
            patchEntry(entry.key, {
              epochStale: true,
              saveState: "error",
              saveError:
                begun.status === "missing-target"
                  ? "The original coding is no longer available. Your text is kept in Unfinished notes."
                  : friendlySaveError("STALE_WORKSPACE"),
              targetMissing: begun.status === "missing-target",
            });
            continue;
          }
          patchEntry(entry.key, {
            epoch: workspace.epoch,
            epochStale: false,
            draftId: begun.record.draft_id,
            ackedRevision: begun.record.revision,
            revision: Math.max(entry.revision, begun.record.revision),
          });
          const live = get().entries[entry.key];
          if (live && live.baseText !== begun.committed_text) {
            if (isDraftDirty(live)) {
              patchEntry(entry.key, {
                saveState: "conflict",
                conflictText: begun.committed_text,
              });
            } else {
              patchEntry(entry.key, {
                baseText: begun.committed_text,
                draftText: begun.committed_text,
                saveState: "clean",
              });
            }
          }
        } catch {
          patchEntry(entry.key, { epochStale: true });
        }
      }
    },

    flushPending: async () => {
      // Departure/export step one: let every in-flight recovery write and
      // commit land (or fail visibly) before snapshotting dirty state. Never
      // time out into an approval — a pending write that cannot finish
      // blocks the action with its own error.
      for (;;) {
        const busy = Object.values(get().entries).filter(
          (entry) => entry.putInFlight || entry.saveInFlight,
        );
        if (busy.length === 0) break;
        await new Promise((resolve) => setTimeout(resolve, 60));
      }
      // One more pass: a put that acked while a newer keystroke queued
      // replays inside runPut's loop, so quiescence here is real.
    },

    refreshRecoveryStatus: async () => {
      try {
        const status = await api.noteRecoveryStatus();
        set({ recoveryAvailable: status.available, recoveryError: status.error });
        if (!status.available) {
          for (const entry of Object.values(get().entries)) {
            if (isDraftDirty(entry) && entry.recoveryState !== "error") {
              patchEntry(entry.key, {
                recoveryState: "unavailable",
                recoveryError:
                  status.error ??
                  "Draft recovery unavailable — your text is held in memory only.",
              });
            }
          }
        }
      } catch {
        // The status probe itself failing is transport trouble, not proof
        // recovery is down: leave the last known state alone.
      }
    },

    removeEntry: (key) =>
      set((state) => {
        const next = { ...state.entries };
        delete next[key];
        return { entries: next };
      }),
  };
});

/** Resolve one recovery record against the open study (Copy/Resume/Discard). */
export async function resolveDraftTarget(
  draftId: string,
): Promise<ResolveNoteDraftTargetResult> {
  return api.resolveNoteDraftTarget(draftId);
}

if (typeof window !== "undefined") {
  (window as unknown as Record<string, unknown>).useNoteDraftStore = useNoteDraftStore;
}
