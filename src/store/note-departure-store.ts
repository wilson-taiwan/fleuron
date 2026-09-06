import { create } from "zustand";
import { api } from "../lib/api";
import { isDraftDirty, useNoteDraftStore, type NoteDraftEntry } from "./note-draft-store";

/**
 * One Promise-based typed choice controller for every departure, replacement
 * and export in the app — study close, opening/replacing a study (including
 * native file-open events), create/join paths that replace the connection,
 * restore/reset, application quit and updater install.
 *
 * Existing confirm-store returns boolean; three choices do not fit in a
 * boolean, and chaining two prompts can approve a different pending action
 * with the first action's answer. So this controller is separate: one dialog
 * host, typed results, and competing departures that wait their turn instead
 * of sharing an answer.
 *
 * Flow per departure:
 *  1. Freeze edits, snapshot attached dirty records across interviews, await
 *     pending recovery/commit IO (which must finish or fail visibly — never
 *     silently time out into an approval).
 *  2. Nothing dirty: continue only after orphan records hold recovery
 *     acknowledgements (an orphan whose backup failed blocks with
 *     Retry / Discard / Cancel). Otherwise offer Save all / Discard / Cancel
 *     (export: Save all and export / Export saved notes only / Cancel).
 *  3. Save all runs serial per-target CAS saves for captured revisions; a
 *     conflict opens the comparison and blocks departure until resolved; a
 *     missing target moves to recovery and is explicitly reported. Failure
 *     stops the action — earlier commits stay saved, nothing is undone.
 *  4. Discard waits for transactional recovery removal, then continues; a
 *     failed discard stays open. Cancel preserves everything.
 */

export type DepartureKind =
  | "close-study"
  | "open-study"
  | "create-study"
  | "restore"
  | "reset"
  | "export"
  | "quit"
  | "update";

export type DepartureDecision =
  | { choice: "save" | "discard" | "cancel" }
  | { choice: "save-export" | "saved-only" };

export interface DepartureResult {
  /** Proceed with the guarded action. */
  proceed: boolean;
  /** The choice that approved it (for export/update branching). */
  choice: DepartureDecision["choice"] | "clean";
  /** Orphan/missing notes kept in local recovery, to report, never drop. */
  keptOrphanCount: number;
}

export interface DepartureLabels {
  title: string;
  body: string;
  saveLabel: string;
  discardLabel: string;
}

const LABELS: Record<DepartureKind, DepartureLabels> = {
  "close-study": {
    title: "Unfinished notes",
    body: "This study has unfinished notes.",
    saveLabel: "Save all",
    discardLabel: "Discard changes",
  },
  "open-study": {
    title: "Unfinished notes",
    body: "The open study has unfinished notes. Opening another study replaces it.",
    saveLabel: "Save all",
    discardLabel: "Discard changes",
  },
  "create-study": {
    title: "Unfinished notes",
    body: "The open study has unfinished notes. Creating a new study replaces it.",
    saveLabel: "Save all",
    discardLabel: "Discard changes",
  },
  restore: {
    title: "Unfinished notes",
    body: "Restoring a backup replaces this study. Unfinished notes belong to the current content.",
    saveLabel: "Save all",
    discardLabel: "Discard changes",
  },
  reset: {
    title: "Unfinished notes",
    body: "Resetting removes coding from this study. Unfinished notes would go with it.",
    saveLabel: "Save all",
    discardLabel: "Discard changes",
  },
  export: {
    title: "Unfinished notes",
    body: "This study has unfinished notes. Exports always use committed notes.",
    saveLabel: "Save all and export",
    discardLabel: "Export saved notes only",
  },
  quit: {
    title: "Unfinished notes",
    body: "Quitting now would leave unfinished notes unsaved.",
    saveLabel: "Save all",
    discardLabel: "Discard changes",
  },
  update: {
    title: "Unfinished notes",
    body: "Installing the update closes this study. Unfinished notes need a decision first.",
    saveLabel: "Save all",
    discardLabel: "Discard changes",
  },
};

interface DeparturePrompt {
  id: string;
  kind: DepartureKind;
  labels: DepartureLabels;
  dirtyKeys: string[];
  dirtyLabels: string[];
  conflictKeys: string[];
  orphanKeys: string[];
  orphanLabels: string[];
  missingKept: number;
  phase: "confirm" | "working" | "conflicts";
  error: string | null;
  resolve: (decision: DepartureDecision["choice"] | "cancelled") => void;
}

interface NoteDepartureStore {
  prompt: DeparturePrompt | null;
  /** One-use close approvals minted by approved departures. */
  closeApprovals: number;
  choose: (choice: DepartureDecision["choice"] | "cancelled") => void;
  grantCloseApproval: () => void;
  consumeCloseApproval: () => boolean;
  requestDeparture: (kind: DepartureKind) => Promise<DepartureResult>;
}

function entryLabel(entry: NoteDraftEntry): string {
  const where =
    entry.kind === "interview"
      ? "Interview note"
      : entry.context.segmentIndex !== null && entry.context.segmentIndex !== undefined
        ? `Passage ${entry.context.segmentIndex + 1}`
        : "Passage note";
  const who = entry.context.participantLabel || entry.interviewId;
  return `${where} · ${who}`;
}

export const useNoteDepartureStore = create<NoteDepartureStore>()((set, get) => {
  // Competing departures wait their turn: each request chains behind the
  // previous one's settlement, then runs its own preflight. No answer is
  // ever reused for a different action.
  let tail: Promise<unknown> = Promise.resolve();

  function setPrompt(patch: Partial<DeparturePrompt>) {
    set((state) => (state.prompt ? { prompt: { ...state.prompt, ...patch } } : state));
  }

  function snapshotDirty(): NoteDraftEntry[] {
    const workspace = useNoteDraftStore.getState().workspace;
    if (!workspace) return [];
    return Object.values(useNoteDraftStore.getState().entries).filter(
      (entry) => entry.projectKey === workspace.projectKey && isDraftDirty(entry),
    );
  }

  function snapshotOrphans(): NoteDraftEntry[] {
    const workspace = useNoteDraftStore.getState().workspace;
    if (!workspace) return [];
    // Orphans: entries whose target is gone but whose text lives on locally.
    // They block only when their recovery acknowledgement is missing —
    // without it there is nowhere to save them to.
    return Object.values(useNoteDraftStore.getState().entries).filter(
      (entry) =>
        entry.projectKey === workspace.projectKey &&
        entry.targetMissing &&
        (entry.recoveryState === "error" || entry.recoveryState === "unavailable"),
    );
  }

  async function runSaveAll(prompt: DeparturePrompt): Promise<boolean> {
    const drafts = useNoteDraftStore.getState();
    setPrompt({ phase: "working", error: null });
    let missingKept = prompt.missingKept;
    for (const key of [...prompt.dirtyKeys]) {
      const entry = useNoteDraftStore.getState().entries[key];
      if (!entry || !isDraftDirty(entry)) continue;
      const ok = await drafts.saveDraft(key);
      const live = useNoteDraftStore.getState().entries[key];
      if (!ok && live?.saveState === "conflict") {
        // Conflict opens the comparison and blocks departure until
        // resolved: stay open on the conflicts phase with the entry's
        // Compare control.
        const remaining = snapshotDirty();
        setPrompt({
          phase: "conflicts",
          dirtyKeys: remaining.map((e) => e.key),
          dirtyLabels: remaining.map(entryLabel),
          conflictKeys: remaining
            .filter((e) => e.saveState === "conflict")
            .map((e) => e.key),
        });
        return false;
      }
      if (!ok && live?.targetMissing) {
        // The original is gone: the text already lives in local recovery,
        // so this entry is settled for departure purposes and reported —
        // never silently dropped, never retried into a save that cannot land.
        missingKept += 1;
        continue;
      }
      if (!ok) {
        // Failure stops the action; earlier commits stay saved and only the
        // remaining drafts stay dirty. Nothing is undone.
        const still = snapshotDirty();
        setPrompt({
          phase: "confirm",
          error: live?.saveError ?? "Could not save every note. The rest stay as drafts.",
          dirtyKeys: still.map((e) => e.key),
          dirtyLabels: still.map(entryLabel),
          missingKept,
        });
        return false;
      }
    }
    // Entries whose target is gone are settled (kept in recovery and
    // reported via missingKept) — only committable dirt blocks.
    const still = snapshotDirty().filter((entry) => !entry.targetMissing);
    if (still.length > 0) {
      setPrompt({
        phase: "conflicts",
        dirtyKeys: still.map((e) => e.key),
        dirtyLabels: still.map(entryLabel),
        conflictKeys: still.filter((e) => e.saveState === "conflict").map((e) => e.key),
        missingKept,
      });
      return false;
    }
    setPrompt({ missingKept });
    return true;
  }

  async function runDiscardAll(prompt: DeparturePrompt): Promise<boolean> {
    const drafts = useNoteDraftStore.getState();
    setPrompt({ phase: "working", error: null });
    for (const key of [...prompt.dirtyKeys]) {
      const entry = useNoteDraftStore.getState().entries[key];
      if (!entry) continue;
      // Explicit discard follows the conditional cleanup rules; a failure
      // stays open with the draft intact.
      const ok = await drafts.discardDraft(key);
      if (!ok) {
        const still = snapshotDirty();
        setPrompt({
          phase: "confirm",
          error: "Could not discard every note. Nothing was deleted.",
          dirtyKeys: still.map((e) => e.key),
          dirtyLabels: still.map(entryLabel),
        });
        return false;
      }
    }
    return true;
  }

  function finish(proceed: boolean, choice: DepartureResult["choice"], keptOrphanCount: number) {
    useNoteDraftStore.getState().setFrozen(false);
    if (proceed) {
      set((state) => ({ closeApprovals: state.closeApprovals + 1 }));
    }
    set({ prompt: null });
    return { proceed, choice, keptOrphanCount };
  }

  async function execute(kind: DepartureKind): Promise<DepartureResult> {
    const drafts = useNoteDraftStore.getState();
    // Freeze background editors for the preflight (the comparison modal
    // used to resolve a conflict stays editable — it recaptures its
    // revision before continuing).
    drafts.setFrozen(true);
    try {
      // Step one: pending IO finishes or surfaces failure. Never time out
      // into an approval.
      await drafts.flushPending();
      await drafts.refreshRecoveryStatus();

      const dirty = snapshotDirty();
      // Orphans whose backup failed are handled FIRST even when other dirt
      // exists: save-all cannot save them (their target is gone), and only a
      // fresh recovery acknowledgement makes them safe to keep.
      const orphans = snapshotOrphans();
      const orphanLabels = orphans.map(entryLabel);
      const committable = dirty.filter(
        (entry) => !orphans.some((orphan) => orphan.key === entry.key),
      );

      if (committable.length === 0 && orphans.length === 0) {
        // Count already-orphaned recovery rows kept locally for the report.
        let kept = 0;
        try {
          const listed = await api.listNoteDrafts();
          kept = listed.filter((d) => d.draft_text !== d.base_text).length;
        } catch {
          kept = 0;
        }
        return finish(true, "clean", kept);
      }

      if (orphans.length > 0) {
        // Orphans with failed backups block: Retry recovery, Discard this
        // draft, or Cancel. They cannot be saved (their target is gone) and
        // closing would strand memory-only text. Retry re-acks the text to
        // local recovery (which needs no live target); then they are kept
        // and reported, and the remaining dirt gets its own prompt.
        const decision = await new Promise<"retry" | "discard" | "cancelled">((resolve) => {
          set({
            prompt: {
              id: `departure-${Date.now()}`,
              kind,
              labels: {
                title: "Unfinished notes need a backup",
                body: "These notes' originals are gone and their local backup failed. They exist only in memory.",
                saveLabel: "Retry recovery",
                discardLabel: "Discard this draft",
              },
              dirtyKeys: [],
              dirtyLabels: [],
              conflictKeys: [],
              orphanKeys: orphans.map((e) => e.key),
              orphanLabels,
              missingKept: 0,
              phase: "confirm",
              error: null,
              resolve: (choice) => {
                if (choice === "save" || choice === "save-export") resolve("retry");
                else if (choice === "discard" || choice === "saved-only") resolve("discard");
                else resolve("cancelled");
              },
            },
          });
        });
        if (decision === "retry") {
          for (const entry of orphans) drafts.retryRecovery(entry.key);
          await drafts.flushPending();
          const still = snapshotOrphans();
          if (still.length > 0) {
            set({ prompt: null });
            drafts.setFrozen(false);
            // Stay open: surface the failure rather than looping silently.
            return { proceed: false, choice: "cancel", keptOrphanCount: 0 };
          }
          // Re-acked orphans are kept, not saved; fall through to whatever
          // committable dirt remains (possibly none).
          const rest = snapshotDirty().filter((entry) => !entry.targetMissing);
          if (rest.length === 0) {
            return finish(true, kind === "export" ? "saved-only" : "save", orphans.length);
          }
        } else if (decision === "discard") {
          for (const entry of orphans) {
            await drafts.discardDraft(entry.key);
          }
          const rest = snapshotDirty().filter((entry) => !entry.targetMissing);
          if (rest.length === 0) {
            return finish(true, kind === "export" ? "saved-only" : "discard", 0);
          }
        } else {
          set({ prompt: null });
          drafts.setFrozen(false);
          return { proceed: false, choice: "cancel", keptOrphanCount: 0 };
        }
      }

      const settled = snapshotDirty().filter((entry) => !entry.targetMissing);
      // A retry may have re-acked orphans into kept-but-dirty rows; they are
      // reported, not re-prompted as committable dirt.
      const keptFromOrphans = orphans.length;
      if (settled.length === 0) {
        return finish(true, kind === "export" ? "saved-only" : "save", keptFromOrphans);
      }

      const labels = LABELS[kind];
      const decision = await new Promise<DepartureDecision["choice"] | "cancelled">((resolve) => {
        set({
          prompt: {
            id: `departure-${Date.now()}`,
            kind,
            labels,
            dirtyKeys: settled.map((e) => e.key),
            dirtyLabels: settled.map(entryLabel),
            conflictKeys: [],
            orphanKeys: orphans.map((e) => e.key),
            orphanLabels,
            missingKept: keptFromOrphans,
            phase: "confirm",
            error: null,
            resolve,
          },
        });
      });

      const prompt = get().prompt;
      if (!prompt || decision === "cancelled") {
        set({ prompt: null });
        drafts.setFrozen(false);
        return { proceed: false, choice: "cancel", keptOrphanCount: 0 };
      }

      if (decision === "saved-only") {
        // Saved-only neither clears drafts nor marks them saved. Orphan
        // exclusions are reported by the caller.
        let kept = prompt.orphanKeys.length;
        try {
          const listed = await api.listNoteDrafts();
          kept = Math.max(
            kept,
            listed.filter((d) => d.draft_text !== d.base_text).length,
          );
        } catch {
          // Report what the prompt already counted.
        }
        return finish(true, "saved-only", kept);
      }

      if (decision === "save" || decision === "save-export") {
        // Loop back through conflicts: each pass saves what it can, and the
        // comparison blocks until resolved or cancelled.
        for (;;) {
          const current = get().prompt;
          if (!current) {
            drafts.setFrozen(false);
            return { proceed: false, choice: "cancel", keptOrphanCount: 0 };
          }
          const done = await runSaveAll(current);
          if (done) {
            const kept = get().prompt?.missingKept ?? 0;
            return finish(true, decision, kept);
          }
          // Not done: either conflicts await the user (stay open on the
          // conflicts phase) or an error returned to confirm. Wait for the
          // next choice: save again, discard, or cancel.
          const next = await new Promise<DepartureDecision["choice"] | "cancelled">((resolve) => {
            const live = get().prompt;
            if (!live) {
              resolve("cancelled");
              return;
            }
            set({
              prompt: { ...live, resolve },
            });
          });
          if (next === "cancelled") {
            set({ prompt: null });
            drafts.setFrozen(false);
            return { proceed: false, choice: "cancel", keptOrphanCount: 0 };
          }
          if (next === "discard" || next === "saved-only") {
            if (next === "saved-only") {
              const kept = get().prompt?.missingKept ?? 0;
              return finish(true, "saved-only", kept);
            }
            const live = get().prompt;
            if (live && (await runDiscardAll(live))) {
              return finish(true, "discard", live.missingKept);
            }
            // Discard failed: runDiscardAll already re-armed confirm.
            continue;
          }
          // "save" again: loop continues with refreshed snapshot keys.
          const refreshed = snapshotDirty();
          setPrompt({
            dirtyKeys: refreshed.map((e) => e.key),
            dirtyLabels: refreshed.map(entryLabel),
          });
        }
      }

      // discard from the confirm phase.
      const live = get().prompt;
      if (live && (await runDiscardAll(live))) {
        return finish(true, "discard", live.missingKept);
      }
      // Discard failed and re-armed confirm; treat as cancelled-open.
      return { proceed: false, choice: "cancel", keptOrphanCount: 0 };
    } catch {
      set({ prompt: null });
      drafts.setFrozen(false);
      return { proceed: false, choice: "cancel", keptOrphanCount: 0 };
    }
  }

  return {
    prompt: null,
    closeApprovals: 0,

    choose: (choice) => {
      get().prompt?.resolve(choice);
    },

    grantCloseApproval: () => {
      set((state) => ({ closeApprovals: state.closeApprovals + 1 }));
    },

    consumeCloseApproval: () => {
      const approvals = get().closeApprovals;
      if (approvals <= 0) return false;
      set({ closeApprovals: approvals - 1 });
      return true;
    },

    requestDeparture: (kind) => {
      const run = tail.then(() => execute(kind));
      // The chain itself never rejects: a failure inside becomes Cancel.
      tail = run.then(
        () => undefined,
        () => undefined,
      );
      return run;
    },
  };
});
