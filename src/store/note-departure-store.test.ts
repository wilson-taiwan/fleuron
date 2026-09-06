import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({
  api: {
    noteRecoveryStatus: vi.fn().mockResolvedValue({ available: true, error: null }),
    listNoteDrafts: vi.fn().mockResolvedValue([]),
    beginNoteDraft: vi.fn(),
    putNoteDraft: vi.fn(),
    discardNoteDraft: vi.fn().mockResolvedValue({ draft_id: "draft-1", revision: 1 }),
    saveNoteDraft: vi.fn(),
    resolveNoteDraftTarget: vi.fn(),
  },
}));

import { api } from "../lib/api";
import { useNoteDepartureStore } from "./note-departure-store";
import { useNoteDraftStore, type NoteDraftEntry } from "./note-draft-store";

function dirtyEntry(key: string, targetId: string, text = "unsaved"): NoteDraftEntry {
  return {
    key,
    projectKey: "proj-1",
    kind: "coding",
    targetId,
    interviewId: "iv-1",
    epoch: "epoch-1",
    draftId: `draft-${targetId}`,
    revision: 1,
    ackedRevision: 1,
    baseText: "",
    draftText: text,
    saveState: "dirty",
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
      coderName: "Ada",
      participantLabel: "P01",
      segmentId: "seg-1",
      segmentIndex: 0,
      charStart: null,
      charEnd: null,
      quoteText: null,
    },
    updatedAt: Date.now(),
  };
}

function seedDirty(keys = ["coding-1"]) {
  const entries: Record<string, NoteDraftEntry> = {};
  for (const targetId of keys) {
    const key = `proj-1::coding::${targetId}`;
    entries[key] = dirtyEntry(key, targetId);
  }
  useNoteDraftStore.setState({
    workspace: { projectKey: "proj-1", epoch: "epoch-1" },
    entries,
    frozen: false,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  useNoteDraftStore.getState().clearWorkspace();
  useNoteDepartureStore.setState({ prompt: null, closeApprovals: 0 });
  vi.mocked(api.listNoteDrafts).mockResolvedValue([]);
  vi.mocked(api.noteRecoveryStatus).mockResolvedValue({ available: true, error: null });
  vi.mocked(api.saveNoteDraft).mockImplementation(async (input: {
    draft_id: string;
    revision: number;
    draft_text: string;
  }) => ({
    status: "saved",
    draft_id: input.draft_id,
    revision: input.revision,
    committed_text: input.draft_text,
    recovery_cleared: true,
  }));
});

describe("note-departure-store", () => {
  it("continues silently when nothing is dirty and recovery is acked", async () => {
    useNoteDraftStore.getState().bindWorkspace("proj-1", "epoch-1");
    const result = await useNoteDepartureStore.getState().requestDeparture("close-study");
    expect(result).toMatchObject({ proceed: true, choice: "clean" });
    expect(useNoteDepartureStore.getState().prompt).toBeNull();
    expect(useNoteDraftStore.getState().frozen).toBe(false);
  });

  it("save-all commits every dirty draft and proceeds", async () => {
    seedDirty(["coding-1", "coding-2"]);
    const request = useNoteDepartureStore.getState().requestDeparture("close-study");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    const prompt = useNoteDepartureStore.getState().prompt!;
    expect(prompt.dirtyKeys).toHaveLength(2);
    useNoteDepartureStore.getState().choose("save");
    const result = await request;
    expect(result).toMatchObject({ proceed: true, choice: "save" });
    expect(vi.mocked(api.saveNoteDraft)).toHaveBeenCalledTimes(2);
    expect(useNoteDepartureStore.getState().prompt).toBeNull();
    expect(useNoteDraftStore.getState().frozen).toBe(false);
  });

  it("discard waits for transactional removal, then proceeds", async () => {
    seedDirty(["coding-1"]);
    const request = useNoteDepartureStore.getState().requestDeparture("close-study");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    useNoteDepartureStore.getState().choose("discard");
    const result = await request;
    expect(result).toMatchObject({ proceed: true, choice: "discard" });
    expect(vi.mocked(api.discardNoteDraft)).toHaveBeenCalledTimes(1);
    expect(useNoteDraftStore.getState().dirtyEntries()).toHaveLength(0);
  });

  it("cancel preserves everything and unfreezes", async () => {
    seedDirty(["coding-1"]);
    const request = useNoteDepartureStore.getState().requestDeparture("close-study");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    expect(useNoteDraftStore.getState().frozen).toBe(true);
    useNoteDepartureStore.getState().choose("cancelled");
    const result = await request;
    expect(result).toMatchObject({ proceed: false, choice: "cancel" });
    expect(useNoteDraftStore.getState().dirtyEntries()).toHaveLength(1);
    expect(useNoteDraftStore.getState().frozen).toBe(false);
    expect(vi.mocked(api.saveNoteDraft)).not.toHaveBeenCalled();
  });

  it("a failed save stops the action; earlier commits stay saved", async () => {
    seedDirty(["coding-1", "coding-2"]);
    vi.mocked(api.saveNoteDraft)
      .mockResolvedValueOnce({
        status: "saved",
        draft_id: "draft-coding-1",
        revision: 1,
        committed_text: "unsaved",
        recovery_cleared: true,
      })
      .mockRejectedValueOnce(new Error("Disk I/O failure"));
    const request = useNoteDepartureStore.getState().requestDeparture("close-study");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    useNoteDepartureStore.getState().choose("save");
    // Failure returns to confirm with the error; cancel out of it.
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt?.error).toMatch(/Could not save/);
    });
    // First draft committed and clean; second still dirty.
    expect(useNoteDraftStore.getState().dirtyEntries()).toHaveLength(1);
    useNoteDepartureStore.getState().choose("cancelled");
    const result = await request;
    expect(result.proceed).toBe(false);
    // Nothing was undone: the first save was never rolled back.
    expect(vi.mocked(api.saveNoteDraft)).toHaveBeenCalledTimes(2);
  });

  it("a conflict blocks departure until compared, then save-all resumes", async () => {
    seedDirty(["coding-1"]);
    vi.mocked(api.saveNoteDraft)
      .mockResolvedValueOnce({ status: "conflict", current_text: "theirs" })
      .mockImplementation(async (input: {
        draft_id: string;
        revision: number;
        draft_text: string;
      }) => ({
        status: "saved",
        draft_id: input.draft_id,
        revision: input.revision,
        committed_text: input.draft_text,
        recovery_cleared: true,
      }));
    const request = useNoteDepartureStore.getState().requestDeparture("close-study");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    useNoteDepartureStore.getState().choose("save");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt?.phase).toBe("conflicts");
    });
    expect(useNoteDepartureStore.getState().prompt?.conflictKeys).toHaveLength(1);
    // Resolve via the entry (as the Compare modal would), then save again.
    const key = "proj-1::coding::coding-1";
    useNoteDraftStore.setState((state) => ({
      entries: {
        ...state.entries,
        [key]: { ...state.entries[key], saveState: "dirty", conflictText: null, baseText: "theirs" },
      },
    }));
    useNoteDepartureStore.getState().choose("save");
    const result = await request;
    expect(result).toMatchObject({ proceed: true, choice: "save" });
  });

  it("a missing target moves to recovery and is explicitly reported", async () => {
    seedDirty(["coding-1"]);
    vi.mocked(api.saveNoteDraft).mockResolvedValueOnce({ status: "missing-target" });
    const request = useNoteDepartureStore.getState().requestDeparture("close-study");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    useNoteDepartureStore.getState().choose("save");
    const result = await request;
    expect(result).toMatchObject({ proceed: true, choice: "save", keptOrphanCount: 1 });
  });

  it("competing departures wait instead of sharing an answer", async () => {
    seedDirty(["coding-1"]);
    const first = useNoteDepartureStore.getState().requestDeparture("close-study");
    const second = useNoteDepartureStore.getState().requestDeparture("export");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt?.kind).toBe("close-study");
    });
    // Answering the first must not resolve the second.
    useNoteDepartureStore.getState().choose("cancelled");
    expect(await first).toMatchObject({ proceed: false });
    // The second runs its own preflight with its own prompt.
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt?.kind).toBe("export");
    });
    useNoteDepartureStore.getState().choose("saved-only");
    expect(await second).toMatchObject({ proceed: true, choice: "saved-only" });
    // Saved-only neither clears drafts nor marks them saved.
    expect(useNoteDraftStore.getState().dirtyEntries()).toHaveLength(1);
    expect(vi.mocked(api.saveNoteDraft)).not.toHaveBeenCalled();
  });

  it("export offers save-and-export and continues to the dialog", async () => {
    seedDirty(["coding-1"]);
    const request = useNoteDepartureStore.getState().requestDeparture("export");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    expect(useNoteDepartureStore.getState().prompt?.labels.saveLabel).toBe("Save all and export");
    useNoteDepartureStore.getState().choose("save-export");
    expect(await request).toMatchObject({ proceed: true, choice: "save-export" });
  });

  it("orphans with failed backups block with retry/discard/cancel", async () => {
    const key = "proj-1::coding::coding-1";
    const orphan: NoteDraftEntry = {
      ...dirtyEntry(key, "coding-1"),
      targetMissing: true,
      recoveryState: "error",
    };
    useNoteDraftStore.setState({
      workspace: { projectKey: "proj-1", epoch: "epoch-1" },
      entries: { [key]: { ...orphan, draftText: "orphan text", baseText: "" } },
      frozen: false,
    });
    const request = useNoteDepartureStore.getState().requestDeparture("quit");
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    expect(useNoteDepartureStore.getState().prompt?.labels.saveLabel).toBe("Retry recovery");
    useNoteDepartureStore.getState().choose("cancelled");
    expect(await request).toMatchObject({ proceed: false });
  });
});
