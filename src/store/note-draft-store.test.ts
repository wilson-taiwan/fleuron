import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/api", () => ({
  api: {
    noteRecoveryStatus: vi.fn().mockResolvedValue({ available: true, error: null }),
    listNoteDrafts: vi.fn().mockResolvedValue([]),
    beginNoteDraft: vi.fn(),
    putNoteDraft: vi.fn(),
    discardNoteDraft: vi.fn(),
    saveNoteDraft: vi.fn(),
    resolveNoteDraftTarget: vi.fn(),
  },
}));

import { api } from "../lib/api";
import {
  draftKey,
  isDraftDirty,
  useNoteDraftStore,
  type DraftTarget,
} from "./note-draft-store";

const WORKSPACE = { projectKey: "proj-1", epoch: "epoch-1" };

const TARGET: DraftTarget = {
  kind: "coding",
  targetId: "coding-1",
  interviewId: "iv-1",
  coderName: "Ada",
  participantLabel: "P01",
  segmentId: "seg-1",
  segmentIndex: 0,
  charStart: null,
  charEnd: null,
  quoteText: "Synthetic passage.",
};

function mockRecord(overrides: Record<string, unknown> = {}) {
  return {
    draft_id: "draft-1",
    project_key: WORKSPACE.projectKey,
    kind: "coding",
    target_id: TARGET.targetId,
    interview_id: TARGET.interviewId,
    coder_name: "Ada",
    base_text: "",
    draft_text: "",
    revision: 0,
    participant_label: "P01",
    segment_id: "seg-1",
    segment_index: 0,
    char_start: null,
    char_end: null,
    quote_text: "Synthetic passage.",
    updated_at: new Date().toISOString(),
    discarded: false,
    ...overrides,
  };
}

/** A manually released promise: no arbitrary sleeps in store tests. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function flushMicrotasks(times = 10) {
  let chain = Promise.resolve();
  for (let i = 0; i < times; i += 1) chain = chain.then(() => undefined);
  return chain;
}

beforeEach(() => {
  vi.clearAllMocks();
  useNoteDraftStore.getState().clearWorkspace();
  useNoteDraftStore.getState().bindWorkspace(WORKSPACE.projectKey, WORKSPACE.epoch);
  vi.mocked(api.noteRecoveryStatus).mockResolvedValue({ available: true, error: null });
});

async function beginClean() {
  vi.mocked(api.beginNoteDraft).mockResolvedValueOnce({
    status: "active",
    record: mockRecord(),
    committed_text: "",
  });
  const entry = await useNoteDraftStore.getState().beginDraft(TARGET);
  return entry;
}

describe("note-draft-store", () => {
  it("derives dirty from text, not from a flag", () => {
    expect(isDraftDirty({ draftText: "a", baseText: "a" })).toBe(false);
    expect(isDraftDirty({ draftText: "a ", baseText: "a" })).toBe(true);
    // Empty strings are deliberate edits, not absences.
    expect(isDraftDirty({ draftText: "", baseText: "" })).toBe(false);
    expect(isDraftDirty({ draftText: "", baseText: "x" })).toBe(true);
  });

  it("begin resumes the acknowledged draft instead of reseeding", async () => {
    vi.mocked(api.beginNoteDraft).mockResolvedValueOnce({
      status: "active",
      record: mockRecord({ draft_text: "half a thought", revision: 3 }),
      committed_text: "",
    });
    const entry = await useNoteDraftStore.getState().beginDraft(TARGET);
    expect(entry.draftText).toBe("half a thought");
    expect(entry.baseText).toBe("");
    expect(entry.saveState).toBe("dirty");
    expect(vi.mocked(api.beginNoteDraft)).toHaveBeenCalledTimes(1);
    // Second begin for the same target focuses without another IPC call.
    const again = await useNoteDraftStore.getState().beginDraft(TARGET);
    expect(again.draftId).toBe(entry.draftId);
    expect(vi.mocked(api.beginNoteDraft)).toHaveBeenCalledTimes(1);
  });

  it("serializes recovery writes and coalesces rapid typing to the latest", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    const first = deferred<{ status: "stored"; record: ReturnType<typeof mockRecord> }>();
    vi.mocked(api.putNoteDraft).mockReturnValueOnce(first.promise as never);
    vi.mocked(api.putNoteDraft).mockImplementation(async (input: {
      draft_id: string;
      expected_revision: number;
      draft_text: string;
    }) => ({
      status: "stored",
      record: mockRecord({ draft_text: input.draft_text, revision: input.expected_revision + 1 }),
    }));

    const store = useNoteDraftStore.getState();
    store.editDraft(key, "v1");
    store.editDraft(key, "v2");
    store.editDraft(key, "v3");
    // One write in flight; the queue holds only the latest.
    expect(vi.mocked(api.putNoteDraft)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.putNoteDraft).mock.calls[0][0]).toMatchObject({
      expected_revision: 0,
      draft_text: "v1",
    });
    first.resolve({
      status: "stored",
      record: mockRecord({ draft_text: "v1", revision: 1 }),
    });
    await flushMicrotasks();
    // The loop replays the coalesced latest with the advanced revision.
    expect(vi.mocked(api.putNoteDraft)).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.putNoteDraft).mock.calls[1][0]).toMatchObject({
      expected_revision: 1,
      draft_text: "v3",
    });
    await flushMicrotasks();
    const entry = useNoteDraftStore.getState().entries[key];
    expect(entry.ackedRevision).toBe(2);
    expect(entry.draftText).toBe("v3");
    expect(entry.recoveryState).toBe("backed-up");
  });

  it("rejects out-of-order acknowledgements (stale put cannot win)", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    // Backend reports the winner's revision; the store adopts it and replays.
    vi.mocked(api.putNoteDraft)
      .mockResolvedValueOnce({
        status: "revision-mismatch",
        record: mockRecord({ draft_text: "winner", revision: 7 }),
      } as never)
      .mockImplementation(async (input: {
        draft_id: string;
        expected_revision: number;
        draft_text: string;
      }) => ({
        status: "stored",
        record: mockRecord({ draft_text: input.draft_text, revision: input.expected_revision + 1 }),
      }));
    useNoteDraftStore.getState().editDraft(key, "loser");
    await flushMicrotasks(20);
    const calls = vi.mocked(api.putNoteDraft).mock.calls;
    expect(calls.length).toBeGreaterThanOrEqual(2);
    expect(calls[1][0]).toMatchObject({ expected_revision: 7, draft_text: "loser" });
    const entry = useNoteDraftStore.getState().entries[key];
    expect(entry.draftText).toBe("loser");
    expect(entry.ackedRevision).toBe(8);
  });

  it("A-then-B during save A keeps B dirty and never reports B saved", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    vi.mocked(api.putNoteDraft).mockImplementation(async (input: {
      draft_id: string;
      expected_revision: number;
      draft_text: string;
    }) => ({
      status: "stored",
      record: mockRecord({ draft_text: input.draft_text, revision: input.expected_revision + 1 }),
    }));
    const store = useNoteDraftStore.getState();
    store.editDraft(key, "A");
    await flushMicrotasks();

    const gate = deferred<{ status: string }>();
    vi.mocked(api.saveNoteDraft).mockReturnValueOnce(gate.promise as never);
    const savePromise = store.saveDraft(key);
    store.editDraft(key, "B");
    // The save captured revision 1 ("A"); memory is already at revision 2.
    expect(vi.mocked(api.saveNoteDraft).mock.calls[0][0]).toMatchObject({
      revision: 1,
      draft_text: "A",
      expected_saved_text: "",
    });
    // Backend: commit of A, cleanup misses (B already bumped the row).
    gate.resolve({
      status: "saved",
      draft_id: "draft-1",
      revision: 1,
      committed_text: "A",
      recovery_cleared: false,
    } as never);
    expect(await savePromise).toBe(true);
    await flushMicrotasks();
    const entry = useNoteDraftStore.getState().entries[key];
    expect(entry.baseText).toBe("A");
    expect(entry.draftText).toBe("B");
    // B stays dirty with its backup queued — it is never reported saved.
    expect(entry.saveState).toBe("dirty");
  });

  it("a rejected write keeps the draft with a sticky error and Retry", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    vi.mocked(api.putNoteDraft).mockImplementation(async (input: {
      expected_revision: number;
      draft_text: string;
    }) => ({
      status: "stored",
      record: mockRecord({ draft_text: input.draft_text, revision: input.expected_revision + 1 }),
    }));
    const store = useNoteDraftStore.getState();
    store.editDraft(key, "unsaved words");
    await flushMicrotasks();
    vi.mocked(api.saveNoteDraft).mockRejectedValueOnce(new Error("Disk I/O failure"));
    const ok = await store.saveDraft(key);
    expect(ok).toBe(false);
    const entry = useNoteDraftStore.getState().entries[key];
    expect(entry.saveState).toBe("error");
    expect(entry.saveError).toMatch(/Could not save note/);
    expect(entry.draftText).toBe("unsaved words");
    // Exactly one notice per failed attempt: no auto-retry happened.
    expect(vi.mocked(api.saveNoteDraft)).toHaveBeenCalledTimes(1);
    // Retry commits the same revision.
    vi.mocked(api.saveNoteDraft).mockResolvedValueOnce({
      status: "saved",
      draft_id: "draft-1",
      revision: 1,
      committed_text: "unsaved words",
      recovery_cleared: true,
    });
    const retried = await store.saveDraft(key);
    expect(retried).toBe(true);
    expect(useNoteDraftStore.getState().entries[key].saveState).toBe("clean");
  });

  it("conflict offers save-mine / use-saved / keep-editing without overwriting", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    vi.mocked(api.putNoteDraft).mockImplementation(async (input: {
      expected_revision: number;
      draft_text: string;
    }) => ({
      status: "stored",
      record: mockRecord({ draft_text: input.draft_text, revision: input.expected_revision + 1 }),
    }));
    const store = useNoteDraftStore.getState();
    store.editDraft(key, "mine");
    await flushMicrotasks();
    vi.mocked(api.saveNoteDraft).mockResolvedValueOnce({
      status: "conflict",
      current_text: "theirs",
    });
    expect(await store.saveDraft(key)).toBe(false);
    expect(useNoteDraftStore.getState().entries[key].saveState).toBe("conflict");
    expect(useNoteDraftStore.getState().entries[key].conflictText).toBe("theirs");

    // Keep editing: both versions stay available, nothing overwritten.
    store.keepEditing(key);
    expect(useNoteDraftStore.getState().entries[key].saveState).toBe("dirty");
    expect(useNoteDraftStore.getState().entries[key].conflictText).toBe("theirs");

    // Save mine re-CASes against the shown version…
    vi.mocked(api.saveNoteDraft).mockResolvedValueOnce({
      status: "saved",
      draft_id: "draft-1",
      revision: 1,
      committed_text: "mine",
      recovery_cleared: true,
    });
    // …but first restore the conflict to exercise saveMineOver.
    useNoteDraftStore.setState((state) => ({
      entries: {
        ...state.entries,
        [key]: { ...state.entries[key], saveState: "conflict", conflictText: "theirs" },
      },
    }));
    expect(await store.saveMineOver(key)).toBe(true);
    const saveCalls = vi.mocked(api.saveNoteDraft).mock.calls;
    expect(saveCalls[saveCalls.length - 1][0]).toMatchObject({
      expected_saved_text: "theirs",
      draft_text: "mine",
    });

    // Use saved version adopts the live text and drops the draft.
    useNoteDraftStore.setState((state) => ({
      entries: {
        ...state.entries,
        [key]: {
          ...state.entries[key],
          saveState: "conflict",
          conflictText: "theirs-v2",
          draftText: "mine",
          baseText: "theirs",
        },
      },
    }));
    vi.mocked(api.discardNoteDraft).mockResolvedValueOnce({ draft_id: "draft-1", revision: 1 });
    expect(await store.useSavedVersion(key)).toBe(true);
    expect(useNoteDraftStore.getState().entries[key]).toBeUndefined();
  });

  it("conflict that moves again refreshes instead of overwriting", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    vi.mocked(api.putNoteDraft).mockImplementation(async (input: {
      expected_revision: number;
      draft_text: string;
    }) => ({
      status: "stored",
      record: mockRecord({ draft_text: input.draft_text, revision: input.expected_revision + 1 }),
    }));
    const store = useNoteDraftStore.getState();
    store.editDraft(key, "mine");
    await flushMicrotasks();
    vi.mocked(api.saveNoteDraft).mockResolvedValueOnce({
      status: "conflict",
      current_text: "theirs-v2",
    });
    useNoteDraftStore.setState((state) => ({
      entries: {
        ...state.entries,
        [key]: { ...state.entries[key], saveState: "conflict", conflictText: "theirs-v1" },
      },
    }));
    expect(await store.saveMineOver(key)).toBe(false);
    expect(useNoteDraftStore.getState().entries[key].conflictText).toBe("theirs-v2");
    expect(useNoteDraftStore.getState().entries[key].draftText).toBe("mine");
  });

  it("discard followed by a late write cannot resurrect the draft", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    const gate = deferred<{ status: "stored"; record: ReturnType<typeof mockRecord> }>();
    vi.mocked(api.putNoteDraft).mockReturnValueOnce(gate.promise as never);
    const store = useNoteDraftStore.getState();
    store.editDraft(key, "doomed");
    // Explicit discard wins over the queued write…
    vi.mocked(api.discardNoteDraft).mockResolvedValueOnce({ draft_id: "draft-1", revision: 0 });
    const discarded = await store.discardDraft(key);
    expect(discarded).toBe(true);
    expect(useNoteDraftStore.getState().entries[key]).toBeUndefined();
    // …and the late acknowledgement finds no entry to resurrect.
    gate.resolve({ status: "stored", record: mockRecord({ draft_text: "doomed", revision: 1 }) });
    await flushMicrotasks();
    expect(useNoteDraftStore.getState().entries[key]).toBeUndefined();
    expect(vi.mocked(api.beginNoteDraft)).toHaveBeenCalledTimes(1);
  });

  it("failed discard leaves the draft intact and visible", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    vi.mocked(api.discardNoteDraft).mockRejectedValueOnce(new Error("Disk I/O failure"));
    const ok = await useNoteDraftStore.getState().discardDraft(key);
    expect(ok).toBe(false);
    expect(useNoteDraftStore.getState().entries[key]).toBeDefined();
  });

  it("stale workspace flags the entry for rebase instead of losing text", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    vi.mocked(api.putNoteDraft).mockResolvedValueOnce({ status: "stale-workspace" });
    useNoteDraftStore.getState().editDraft(key, "stranded");
    await flushMicrotasks();
    const entry = useNoteDraftStore.getState().entries[key];
    expect(entry.epochStale).toBe(true);
    expect(entry.draftText).toBe("stranded");
  });

  it("snapshots adopt into clean entries and conflict dirty ones", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    const store = useNoteDraftStore.getState();
    store.adoptSnapshotText("coding", TARGET.targetId, "synced text");
    expect(useNoteDraftStore.getState().entries[key].draftText).toBe("synced text");
    expect(useNoteDraftStore.getState().entries[key].saveState).toBe("clean");

    store.editDraft(key, "local divergence");
    store.adoptSnapshotText("coding", TARGET.targetId, "synced text v2");
    const entry = useNoteDraftStore.getState().entries[key];
    expect(entry.draftText).toBe("local divergence");
    expect(entry.saveState).toBe("conflict");
    expect(entry.conflictText).toBe("synced text v2");
  });

  it("same text on different targets stays separate; rapid same-target opens focus", async () => {
    vi.mocked(api.beginNoteDraft).mockImplementation(async (input: { target_id: string }) => ({
      status: "active",
      record: mockRecord({ draft_id: `draft-${input.target_id}`, target_id: input.target_id }),
      committed_text: "",
    }));
    const store = useNoteDraftStore.getState();
    const first = await store.beginDraft(TARGET);
    const second = await store.beginDraft({ ...TARGET, targetId: "coding-2" });
    expect(first.draftId).not.toBe(second.draftId);
    const refocus = await store.beginDraft({ ...TARGET, targetId: "coding-2" });
    expect(refocus.draftId).toBe(second.draftId);
    expect(vi.mocked(api.beginNoteDraft)).toHaveBeenCalledTimes(2);
  });

  it("acknowledged recovery survives a reload as a dirty retained draft", async () => {
    // Crash boundary: only acknowledged writes come back. The reload path
    // re-begins and resumes the acknowledged text, still uncommitted.
    vi.mocked(api.beginNoteDraft).mockResolvedValueOnce({
      status: "active",
      record: mockRecord({ draft_text: "acknowledged fragment", revision: 4 }),
      committed_text: "",
    });
    const entry = await useNoteDraftStore.getState().beginDraft(TARGET);
    expect(entry.draftText).toBe("acknowledged fragment");
    expect(entry.saveState).toBe("dirty");
    expect(entry.ackedRevision).toBe(4);
  });

  it("flushPending waits for in-flight writes instead of timing out", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    const gate = deferred<{ status: "stored"; record: ReturnType<typeof mockRecord> }>();
    vi.mocked(api.putNoteDraft).mockReturnValueOnce(gate.promise as never);
    useNoteDraftStore.getState().editDraft(key, "waiting");
    let flushed = false;
    const flush = useNoteDraftStore.getState().flushPending().then(() => {
      flushed = true;
    });
    await flushMicrotasks(3);
    expect(flushed).toBe(false);
    gate.resolve({ status: "stored", record: mockRecord({ draft_text: "waiting", revision: 1 }) });
    await flush;
    expect(flushed).toBe(true);
  });

  it("recovery failure keeps editing in memory with a persistent warning", async () => {
    await beginClean();
    const key = draftKey(WORKSPACE.projectKey, "coding", TARGET.targetId);
    vi.mocked(api.putNoteDraft).mockRejectedValueOnce(new Error("Disk I/O failure"));
    useNoteDraftStore.getState().editDraft(key, "memory only");
    await flushMicrotasks();
    const entry = useNoteDraftStore.getState().entries[key];
    expect(entry.recoveryState).toBe("error");
    expect(entry.draftText).toBe("memory only");
    // Retry replays the latest text.
    vi.mocked(api.putNoteDraft).mockImplementation(async (input: {
      expected_revision: number;
      draft_text: string;
    }) => ({
      status: "stored",
      record: mockRecord({ draft_text: input.draft_text, revision: input.expected_revision + 1 }),
    }));
    useNoteDraftStore.getState().retryRecovery(key);
    await flushMicrotasks(20);
    expect(useNoteDraftStore.getState().entries[key].recoveryState).toBe("backed-up");
  });
});
