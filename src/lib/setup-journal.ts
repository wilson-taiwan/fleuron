export type SetupStage =
  | "validate"
  | "created_folder"
  | "template_initialized"
  | "transcript_imported"
  | "bound_to_group"
  | "initial_sync_done"
  | "completed";

export interface SetupJournalEntry {
  operationId: string;
  kind: "new_local" | "new_shared" | "join_group";
  title: string;
  coderName: string;
  canonicalFolder?: string;
  projectId?: string;
  completedStages: SetupStage[];
  pendingStage: SetupStage;
  error?: string;
  updatedAt: number;
}

const ACTIVE_JOURNAL_KEY = "fleuron_active_setup_journal";
const JOURNAL_PREFIX = "fleuron_setup_journal_";

export const setupJournal = {
  start(
    kind: "new_local" | "new_shared" | "join_group",
    params: {
      title: string;
      coderName: string;
      projectId?: string;
      canonicalFolder?: string;
    },
  ): SetupJournalEntry {
    const operationId = `setup-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    const entry: SetupJournalEntry = {
      operationId,
      kind,
      title: params.title,
      coderName: params.coderName,
      projectId: params.projectId,
      canonicalFolder: params.canonicalFolder,
      completedStages: ["validate"],
      pendingStage: "created_folder",
      updatedAt: Date.now(),
    };
    try {
      localStorage.setItem(
        `${JOURNAL_PREFIX}${operationId}`,
        JSON.stringify(entry),
      );
      localStorage.setItem(ACTIVE_JOURNAL_KEY, operationId);
    } catch {
      // Storage unavailable or quota exceeded
    }
    return entry;
  },

  update(
    operationId: string,
    updates: Partial<SetupJournalEntry>,
  ): SetupJournalEntry | null {
    try {
      const raw = localStorage.getItem(`${JOURNAL_PREFIX}${operationId}`);
      if (!raw) return null;
      const current = JSON.parse(raw) as SetupJournalEntry;
      const merged: SetupJournalEntry = {
        ...current,
        ...updates,
        updatedAt: Date.now(),
      };
      localStorage.setItem(
        `${JOURNAL_PREFIX}${operationId}`,
        JSON.stringify(merged),
      );
      return merged;
    } catch {
      return null;
    }
  },

  markStageComplete(
    operationId: string,
    stage: SetupStage,
    nextPending: SetupStage,
    extra?: Partial<SetupJournalEntry>,
  ): SetupJournalEntry | null {
    try {
      const raw = localStorage.getItem(`${JOURNAL_PREFIX}${operationId}`);
      if (!raw) return null;
      const current = JSON.parse(raw) as SetupJournalEntry;
      const stages = new Set(current.completedStages);
      stages.add(stage);
      const merged: SetupJournalEntry = {
        ...current,
        ...extra,
        completedStages: Array.from(stages),
        pendingStage: nextPending,
        updatedAt: Date.now(),
      };
      localStorage.setItem(
        `${JOURNAL_PREFIX}${operationId}`,
        JSON.stringify(merged),
      );
      return merged;
    } catch {
      return null;
    }
  },

  getActive(): SetupJournalEntry | null {
    try {
      const activeId = localStorage.getItem(ACTIVE_JOURNAL_KEY);
      if (!activeId) return null;
      const raw = localStorage.getItem(`${JOURNAL_PREFIX}${activeId}`);
      if (!raw) return null;
      return JSON.parse(raw) as SetupJournalEntry;
    } catch {
      return null;
    }
  },

  clear(operationId?: string): void {
    try {
      const id = operationId || localStorage.getItem(ACTIVE_JOURNAL_KEY);
      if (id) {
        localStorage.removeItem(`${JOURNAL_PREFIX}${id}`);
      }
      if (
        !operationId ||
        localStorage.getItem(ACTIVE_JOURNAL_KEY) === operationId
      ) {
        localStorage.removeItem(ACTIVE_JOURNAL_KEY);
      }
    } catch {
      // Best-effort cleanup
    }
  },
};
