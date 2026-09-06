import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./confirm-store", () => ({
  appConfirm: vi.fn().mockResolvedValue(true),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: vi.fn(),
  revealItemInDir: vi.fn(),
}));
vi.mock("../lib/api", () => ({
  api: {
    readTextFile: vi.fn(),
    importSegments: vi.fn(),
    applyCodes: vi.fn(),
    listCodedSegments: vi.fn().mockResolvedValue([]),
    deleteCodedSegment: vi.fn().mockResolvedValue(undefined),
    refreshCodedCount: vi.fn(),
    countCodedSegments: vi.fn().mockResolvedValue(0),
    getSegments: vi.fn().mockResolvedValue([]),
    saveWorkspaceState: vi.fn().mockResolvedValue(undefined),
    listInterviews: vi.fn().mockResolvedValue([]),
    listCodes: vi.fn().mockResolvedValue([]),
    createCode: vi.fn(),
    ensureCodeAndApply: vi.fn(),
    mutateCodingEdge: vi.fn(),
    patchCodingMemo: vi.fn(),
    updateCode: vi.fn().mockResolvedValue({} as never),
    listRecentProjects: vi.fn().mockResolvedValue([]),
    closeProject: vi.fn().mockResolvedValue(undefined),
    exportProject: vi.fn(),
    createHandoffBundle: vi.fn(),
    recordSave: vi.fn().mockResolvedValue(undefined),
    getProjectInfo: vi.fn(),
    adoptProjectCoder: vi.fn(async (_from: string, to: string) => ({
      ...baseProject,
      coders: [to],
    })),
    recordRecentProject: vi.fn().mockResolvedValue([]),
    claimProject: vi.fn(),
    getHandoffDigest: vi.fn(),
    getAppPreferences: vi.fn(),
    setAppPreferences: vi.fn(),
    openProject: vi.fn(),
    isHandoffBundle: vi.fn().mockResolvedValue(false),
    inspectHandoffBundle: vi.fn(),
    openHandoffBundle: vi.fn(),
    restoreWorkspace: vi.fn(),
    listSegmentReviews: vi.fn().mockResolvedValue([]),
    setSegmentReviewed: vi.fn().mockResolvedValue(undefined),
    setSegmentSpeaker: vi.fn().mockResolvedValue([]),
    restoreSegmentSpeakers: vi.fn().mockResolvedValue(undefined),
    getWorkspaceState: vi.fn().mockResolvedValue({
      active_interview_id: null,
      selected_segment_id: null,
      active_coder: null,
    }),
    // Note-draft checked-write contract (mirrors src/lib/api.ts). Most
    // project-store tests run unbound, so these stay uncalled; they exist so
    // bound flows degrade to explicit rejections instead of TypeErrors.
    noteRecoveryStatus: vi.fn().mockResolvedValue({ available: true, error: null }),
    listNoteDrafts: vi.fn().mockResolvedValue([]),
    beginNoteDraft: vi.fn(),
    putNoteDraft: vi.fn(),
    discardNoteDraft: vi.fn(),
    saveNoteDraft: vi.fn(),
    resolveNoteDraftTarget: vi.fn(),
    setRecoveryRootForSelftest: vi.fn().mockResolvedValue(undefined),
    approveUpdateDeparture: vi.fn(),
    completeNoteDeparture: vi.fn(),
  },
}));

// The app-owned confirm system replaced the native dialog plugin in stores.
import { appConfirm } from "./confirm-store";
import { api } from "../lib/api";
import { useProjectStore } from "./project-store";
import { useNoteDraftStore } from "./note-draft-store";
import { computeInterviewCodedCount } from "../lib/store-helpers";
import type { Code, CodedSegment, ProjectInfo } from "../lib/types";

const g = globalThis as unknown as { window?: unknown };
if (!g.window) g.window = globalThis;

const baseProject: ProjectInfo = {
  path: "/tmp/test.qcproj",
  title: "Test",
  methodology: "reflexive-ta",
  coders: ["Alice"],
  last_saved_by: null,
  last_saved_at: null,
};

function resetStore(overrides: Record<string, unknown> = {}) {
  useProjectStore.setState({
    project: baseProject,
    activeCoder: "Alice",
    interviews: [],
    activeInterviewId: null,
    segments: [],
    codes: [],
    codedSegments: [],
    totalCodedCount: 0,
    interviewCodedCount: 0,
    selectedSegmentId: null,
    selectedCodeIds: [],
    noteEditorCodingId: null,
    hubMemo: "",
    savedHubMemo: "",
    showRecoveryPanel: false,
    hubMemoDirty: false,
    toasts: [],
    // Baseline for most tests is "somebody has said who they are"; the
    // identity gate has its own tests below.
    coderConfirmed: true,
    showIdentityPrompt: false,
    undoStack: [],
    redoStack: [],
    ...overrides,
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  resetStore();
  useNoteDraftStore.getState().clearWorkspace();
  vi.mocked(api.mutateCodingEdge).mockImplementation(async (input) => {
    const state = useProjectStore.getState();
    const charStart = input.char_start ?? null;
    const charEnd = input.char_end ?? null;
    const existing = state.codedSegments.find(
      (coding) =>
        coding.interview_id === input.interview_id &&
        coding.segment_id === input.segment_id &&
        coding.coder_name === input.coder_name &&
        coding.char_start === charStart &&
        coding.char_end === charEnd,
    );
    const nextIds = input.present
      ? existing?.code_ids.includes(input.code_id)
        ? existing.code_ids
        : [...(existing?.code_ids ?? []), input.code_id]
      : (existing?.code_ids ?? []).filter((codeId) => codeId !== input.code_id);
    const segment = state.segments.find((entry) => entry.id === input.segment_id);
    const codedSegment = nextIds.length
      ? {
          id: existing?.id ?? "synthetic-coding",
          interview_id: input.interview_id,
          segment_id: input.segment_id,
          code_ids: nextIds,
          coder_name: input.coder_name,
          memo: existing?.memo ?? null,
          char_start: charStart,
          char_end: charEnd,
          quote_text: segment?.text ?? "",
          block_id: segment?.block_id ?? null,
          timestamp_start: segment?.timestamp_start ?? "",
          participant_label: "Synthetic",
        }
      : null;
    return {
      coded_segment: codedSegment,
      changed_edge: {
        interview_id: input.interview_id,
        segment_id: input.segment_id,
        code_id: input.code_id,
        char_start: charStart,
        char_end: charEnd,
        present: input.present,
      },
    } as never;
  });
  vi.mocked(api.patchCodingMemo).mockImplementation(async (input) => {
    const row = useProjectStore
      .getState()
      .codedSegments.find((coding) => coding.id === input.coded_segment_id);
    if (!row) throw new Error("missing synthetic coding row");
    return { ...row, memo: input.memo ?? null };
  });
});

describe("F1 — setSelectedSegmentId code hydration", () => {
  it("hydrates whole-segment codes from an existing coding by the active coder", () => {
    const segments = [
      { id: "seg1", interview_id: "iv1", segment_index: 0, speaker: "A", timestamp_start: "00:00:01.000", timestamp_end: null, text: "one", block_id: null, section_tag: null },
      { id: "seg2", interview_id: "iv1", segment_index: 1, speaker: "A", timestamp_start: "00:00:02.000", timestamp_end: null, text: "two", block_id: null, section_tag: null },
    ];
    const coded: CodedSegment[] = [
      {
        id: "cs1", interview_id: "iv1", segment_id: "seg2",
        code_ids: ["c1"], coder_name: "Alice", memo: "existing memo",
        char_start: null, char_end: null, quote_text: "two",
        block_id: null, timestamp_start: "00:00:02.000", participant_label: "P",
      },
    ];
    resetStore({ segments, codedSegments: coded, selectedSegmentId: "seg1" });

    useProjectStore.getState().setSelectedSegmentId("seg2");

    const s = useProjectStore.getState();
    expect(s.selectedSegmentId).toBe("seg2");
    expect(s.selectedCodeIds).toEqual(["c1"]);
  });

  it("clears codes when navigating to a segment with no coding", () => {
    const segments = [
      { id: "seg1", interview_id: "iv1", segment_index: 0, speaker: "A", timestamp_start: "00:00:01.000", timestamp_end: null, text: "one", block_id: null, section_tag: null },
      { id: "seg2", interview_id: "iv1", segment_index: 1, speaker: "A", timestamp_start: "00:00:02.000", timestamp_end: null, text: "two", block_id: null, section_tag: null },
    ];
    resetStore({ segments, codedSegments: [], selectedSegmentId: "seg1", selectedCodeIds: ["c1"] });

    useProjectStore.getState().setSelectedSegmentId("seg2");

    const s = useProjectStore.getState();
    expect(s.selectedCodeIds).toEqual([]);
  });
});

describe("v0.27 exact-ID coding regressions", () => {
  function code(id: string, name: string): Code {
    return {
      id,
      name,
      definition: null,
      inclusion_criteria: null,
      exclusion_criteria: null,
      example: null,
      parent_id: null,
      color: "#8a6410",
      sort_order: 0,
      is_retired: false,
      usage_count: 0,
    };
  }

  function readyForCreate(activeCoder = "Mac") {
    resetStore({
      activeInterviewId: "iv-1",
      selectedSegmentId: "seg-1",
      activeCoder,
      coderConfirmed: true,
      segments: [{
        id: "seg-1",
        interview_id: "iv-1",
        segment_index: 0,
        speaker: "Synthetic",
        timestamp_start: "00:00:00.000",
        timestamp_end: null,
        text: "synthetic passage",
        block_id: null,
        section_tag: null,
      }],
      codes: [],
      codedSegments: [],
      selectedCodeIds: [],
    });
    vi.mocked(api.applyCodes).mockResolvedValue({
      id: "coding-1",
      interview_id: "iv-1",
      segment_id: "seg-1",
      code_ids: [],
      coder_name: activeCoder,
      memo: null,
      char_start: null,
      char_end: null,
      quote_text: "",
      block_id: null,
      timestamp_start: "00:00:00.000",
      participant_label: "Synthetic",
    } as CodedSegment);
  }

  function ensured(code: Code, coderName = "Mac") {
    return {
      code,
      coded_segment: {
        id: `coding-${code.id}`,
        interview_id: "iv-1",
        segment_id: "seg-1",
        code_ids: [code.id],
        coder_name: coderName,
        memo: null,
        char_start: null,
        char_end: null,
        quote_text: "synthetic passage",
        block_id: null,
        timestamp_start: "00:00:00.000",
        participant_label: "Synthetic",
      },
      created: true,
      changed_edges: [{
        interview_id: "iv-1",
        segment_id: "seg-1",
        code_id: code.id,
        char_start: null,
        char_end: null,
        present: true,
      }],
    };
  }

  it("uses the create command's exact code ID when a stale reload puts a remote code first", async () => {
    const windows = code("windows-1", "Windows 1");
    const mac = code("mac-1", "Mac 1");
    readyForCreate();
    vi.mocked(api.ensureCodeAndApply).mockResolvedValue(ensured(mac));
    vi.mocked(api.listCodes).mockResolvedValue([windows, mac]);

    await useProjectStore.getState().createCodeAndApply("Mac 1");

    expect(api.ensureCodeAndApply).toHaveBeenCalledWith(expect.objectContaining({
      name: "Mac 1",
    }));
    expect(useProjectStore.getState().selectedCodeIds).toEqual(["mac-1"]);
    expect(useProjectStore.getState().codedSegments[0]?.code_ids).toEqual(["mac-1"]);
    expect(api.applyCodes).not.toHaveBeenCalled();
  });

  it.each([
    ["distinct accounts", "Mac"],
    ["one account on two devices", "Same person"],
  ])("preserves all codebook rows and applies intended IDs for the %s chain", async (_mode, coder) => {
    const windows1 = code("windows-1", "Windows 1");
    const mac1 = code("mac-1", "Mac 1");
    const windows2 = code("windows-2", "Windows 2");
    const remoteCodes: Code[] = [windows1];
    readyForCreate(coder);
    vi.mocked(api.ensureCodeAndApply).mockImplementation(async ({ name, coder_name }) => {
      const created = name === "Mac 1" ? mac1 : windows2;
      remoteCodes.push(created);
      return ensured(created, coder_name) as never;
    });
    vi.mocked(api.listCodes).mockImplementation(async () => [...remoteCodes]);

    await useProjectStore.getState().createCodeAndApply("Mac 1");
    readyForCreate(coder);
    await useProjectStore.getState().createCodeAndApply("Windows 2");

    expect(remoteCodes.map((entry) => entry.id)).toEqual([
      "windows-1",
      "mac-1",
      "windows-2",
    ]);
    expect(vi.mocked(api.ensureCodeAndApply).mock.calls.map((call) => call[0].name)).toEqual([
      "Mac 1",
      "Windows 2",
    ]);
    expect(vi.mocked(api.ensureCodeAndApply).mock.calls.map((call) => call[0].segment_id)).toEqual([
      "seg-1",
      "seg-1",
    ]);
  });
});

describe("v0.27 coherent live-workspace reconciliation", () => {
  function snapshot(overrides: Record<string, unknown> = {}) {
    return {
      project: baseProject,
      interviews: [{
        id: "iv1",
        participant_label: "Synthetic",
        interview_date: null,
        modality: null,
        diagnosis_notes: null,
        interviewers: [],
        hub_memo: "remote hub",
        audio_path: null,
        segment_count: 1,
        remote_segment_count: null,
      }],
      codes: [{ id: "remote-code", name: "Remote", color: "#8a6410", sort_order: 0, is_retired: false, usage_count: 1 }],
      retired_codes: [],
      active_interview_id: "iv1",
      selected_segment_id: "seg1",
      segments: [{
        id: "seg1",
        interview_id: "iv1",
        segment_index: 0,
        speaker: "Synthetic",
        timestamp_start: "00:00:00.000",
        timestamp_end: null,
        text: "synthetic passage",
        block_id: null,
        section_tag: null,
      }],
      coded_segments: [{
        id: "remote-coding",
        interview_id: "iv1",
        segment_id: "seg1",
        code_ids: ["remote-code"],
        coder_name: "Alice",
        memo: null,
        char_start: 2,
        char_end: 8,
        quote_text: "ntheti",
        block_id: null,
        timestamp_start: "00:00:00.000",
        participant_label: "Synthetic",
      }],
      pending_coded_count: 0,
      coded_count: 1,
      conflicts: [],
      sync_status: {
        protocol: 2,
        generation: "generation-synthetic",
        local_sequence: 4,
        observed_head: 4,
        outbox_count: 0,
        blocked_count: 0,
        unresolved_conflict_count: 0,
      },
      local_revision: 4,
      ...overrides,
    } as any;
  }

  it("commits durable slices together while preserving a selected span, memo draft, and undo history", () => {
    const undo = { label: "synthetic", undo: async () => {}, redo: async () => {} };
    resetStore({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg1",
      activeCoder: "Alice",
      codes: [{ id: "old-code", name: "Old", color: "#111", sort_order: 0, is_retired: false, usage_count: 1 }],
      selectedCodeIds: ["old-code"],
      pendingSelection: { segmentId: "seg1", start: 2, end: 8, text: "ntheti" },
      hubMemo: "unsaved local memo",
      savedHubMemo: "saved memo",
      hubMemoDirty: true,
      undoStack: [undo],
      redoStack: [undo],
    });
    let commits = 0;
    const unsubscribe = useProjectStore.subscribe(() => { commits += 1; });

    useProjectStore.getState().reconcileLiveWorkspace(snapshot());
    unsubscribe();

    const state = useProjectStore.getState();
    expect(commits).toBe(1);
    expect(state.codes.map((code) => code.id)).toEqual(["remote-code"]);
    expect(state.codedSegments.map((coding) => coding.id)).toEqual(["remote-coding"]);
    expect(state.pendingSelection).toEqual({ segmentId: "seg1", start: 2, end: 8, text: "ntheti" });
    expect(state.selectedCodeIds).toEqual(["remote-code"]);
    expect(state.hubMemo).toBe("unsaved local memo");
    expect(state.hubMemoDirty).toBe(true);
    expect(state.undoStack).toEqual([undo]);
    expect(state.redoStack).toEqual([undo]);
    expect(state.liveSyncStatus?.observed_head).toBe(4);
  });

  it("falls back from a remotely deleted active interview without dangling selection or note IDs", () => {
    resetStore({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg1",
      noteEditorCodingId: "deleted-coding",
      codedSegments: [{
        id: "deleted-coding",
        interview_id: "iv1",
        segment_id: "seg1",
        code_ids: ["old-code"],
        coder_name: "Alice",
        memo: null,
        char_start: null,
        char_end: null,
        quote_text: "synthetic",
        block_id: null,
        timestamp_start: "00:00:00.000",
        participant_label: "Synthetic",
      }],
    });
    const next = snapshot({
      interviews: [{
        id: "iv2",
        participant_label: "Next",
        interview_date: null,
        modality: null,
        diagnosis_notes: null,
        interviewers: [],
        hub_memo: null,
        audio_path: null,
        segment_count: 1,
        remote_segment_count: null,
      }],
      active_interview_id: "iv2",
      selected_segment_id: "seg2",
      segments: [{
        id: "seg2",
        interview_id: "iv2",
        segment_index: 0,
        speaker: "Synthetic",
        timestamp_start: "00:00:00.000",
        timestamp_end: null,
        text: "next passage",
        block_id: null,
        section_tag: null,
      }],
      coded_segments: [],
    });

    useProjectStore.getState().reconcileLiveWorkspace(next);

    const state = useProjectStore.getState();
    expect(state.activeInterviewId).toBe("iv2");
    expect(state.selectedSegmentId).toBe("seg2");
    expect(state.noteEditorCodingId).toBeNull();
    expect(state.selectedCodeIds).toEqual([]);
  });

  it("preserves an intentional null selection across a same-interview live snapshot", () => {
    resetStore({
      activeInterviewId: "iv1",
      selectedSegmentId: null,
      selectedCodeIds: ["remote-code"],
      codes: [{
        id: "remote-code",
        name: "Remote",
        color: "#111",
        sort_order: 0,
        is_retired: false,
        usage_count: 1,
      }],
    });

    useProjectStore.getState().reconcileLiveWorkspace(snapshot());

    const state = useProjectStore.getState();
    expect(state.activeInterviewId).toBe("iv1");
    expect(state.selectedSegmentId).toBeNull();
    expect(state.pendingSelection).toBeNull();
    expect(state.selectedCodeIds).toEqual([]);
  });

  it("falls back from a deleted passage without carrying the old passage's code ids", () => {
    resetStore({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg-gone",
      selectedCodeIds: ["remote-code"],
      codes: [{
        id: "remote-code",
        name: "Remote",
        color: "#111",
        sort_order: 0,
        is_retired: false,
        usage_count: 1,
      }],
      pendingSelection: { segmentId: "seg-gone", start: 0, end: 4, text: "gone" },
    });

    useProjectStore.getState().reconcileLiveWorkspace(snapshot());

    const state = useProjectStore.getState();
    expect(state.selectedSegmentId).toBe("seg1");
    expect(state.pendingSelection).toBeNull();
    expect(state.selectedCodeIds).toEqual([]);
  });
});

describe("F2 — toggleCodeOnTarget error surfacing", () => {
  it("shows an error toast when the edge mutation rejects", async () => {
    const segments = [
      { id: "seg1", interview_id: "iv1", segment_index: 0, speaker: "A", timestamp_start: "00:00:01.000", timestamp_end: null, text: "one", block_id: null, section_tag: null },
    ];
    resetStore({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg1",
      segments,
      codes: [{ id: "c1", name: "test", color: "#8a6410", sort_order: 0, is_retired: false, usage_count: 0 }],
      selectedCodeIds: [],
    });
    vi.mocked(api.mutateCodingEdge).mockRejectedValueOnce(new Error("boom"));

    await useProjectStore.getState().toggleCodeOnTarget("c1");

    const s = useProjectStore.getState();
    expect(s.toasts.some((t) => t.type === "error" && t.text.includes("boom"))).toBe(true);
    expect(vi.mocked(api.mutateCodingEdge)).toHaveBeenCalledTimes(1);
  });
});

describe("opening and passage-note state", () => {
  it("hydrates the workspace atomically from openProject snapshot", async () => {
    const mockSnapshot = {
      project: baseProject,
      codes: [{ id: "c1", name: "Concept", color: "#1f7a5e", sort_order: 0, is_retired: false, usage_count: 1 }],
      interviews: [{ id: "iv1", participant_label: "P01", interview_date: "2026-01-01", modality: "in-person", diagnosis_notes: null, interviewers: ["Alice"], hub_memo: "memo", audio_path: null, segment_count: 1, remote_segment_count: null }],
      workspace: { active_interview_id: "iv1", selected_segment_id: "seg1", active_coder: "Alice" },
      active_interview_id: "iv1",
      selected_segment_id: "seg1",
      segments: [{ id: "seg1", interview_id: "iv1", segment_index: 0, speaker: "P", timestamp_start: "00:00:01.000", timestamp_end: null, text: "text", block_id: null, section_tag: null }],
      coded_segments: [{ id: "cs1", interview_id: "iv1", segment_id: "seg1", code_ids: ["c1"], coder_name: "Alice", memo: "note", char_start: null, char_end: null }],
      total_coded_count: 1,
      recent_code_ids: ["c1"],
      diagnostics: {
        schema_version: 4,
        counts: { codes: 1, interviews: 1, segments: 1, coded_segments: 1 },
        timings_ms: { connection: 1, schema: 1, snapshot_queries: 1, total: 3 },
      },
    };

    vi.mocked(api.openProject).mockResolvedValueOnce(mockSnapshot as any);
    vi.mocked(api.getAppPreferences).mockResolvedValue({ coder_identities: {} } as never);
    // Same-path reopen is a no-op once a study is open (it would only rotate
    // the workspace epoch and strand drafts), so start closed here.
    useProjectStore.setState({ project: null });

    await useProjectStore.getState().openProject(baseProject.path);

    expect(useProjectStore.getState().loading).toBe(false);
    expect(useProjectStore.getState().activeInterviewId).toBe("iv1");
    expect(useProjectStore.getState().segments).toHaveLength(1);
    expect(useProjectStore.getState().codedSegments).toHaveLength(1);
    expect(useProjectStore.getState().recentCodeIds).toEqual(["c1"]);
  });

  it("autosaves a note against its existing coding without reloading all codings", async () => {
    const coding: CodedSegment = {
      id: "cs1",
      interview_id: "iv1",
      segment_id: "seg1",
      code_ids: ["c1"],
      coder_name: "Alice",
      memo: "old note",
      char_start: 4,
      char_end: 12,
      quote_text: "rehearsed",
      block_id: null,
      timestamp_start: "00:00:01.000",
      participant_label: "P",
    };
    const updated = { ...coding, memo: "new note" };
    resetStore({ codedSegments: [coding] });
    vi.mocked(api.patchCodingMemo).mockResolvedValueOnce(updated);

    await useProjectStore.getState().saveMemoForCoding("cs1", "new note", { silent: true });

    expect(api.patchCodingMemo).toHaveBeenCalledWith({
      coded_segment_id: "cs1",
      memo: "new note",
    });
    expect(api.listCodedSegments).not.toHaveBeenCalled();
    expect(useProjectStore.getState().codedSegments[0].memo).toBe("new note");
  });
});

describe("F3 — importVtt re-import confirmation", () => {
  it("cancels when the user declines the confirm and skips the backend import", async () => {
    const coded: CodedSegment[] = [
      {
        id: "cs1", interview_id: "iv1", segment_id: "seg1",
        code_ids: ["c1"], coder_name: "Alice", memo: "m",
        char_start: null, char_end: null, quote_text: "x",
        block_id: "q001", timestamp_start: "00:00:01.000", participant_label: "P",
      },
    ];
    resetStore({ activeInterviewId: "iv1", codedSegments: coded });
    vi.mocked(appConfirm).mockResolvedValueOnce(false);

    const count = await useProjectStore.getState().importVtt("/tmp/foo.vtt");

    expect(count).toBe(0);
    expect(vi.mocked(api.readTextFile)).not.toHaveBeenCalled();
    expect(vi.mocked(api.importSegments)).not.toHaveBeenCalled();
    const s = useProjectStore.getState();
    expect(s.toasts.some((t) => t.type === "info" && t.text.includes("Import cancelled"))).toBe(true);
  });
});

describe("B2 — setSelectedSegmentId debounce", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("coalesces rapid segment navigation into a single saveWorkspaceState call", () => {
    vi.mocked(api.saveWorkspaceState).mockClear();
    const segments = [
      { id: "seg1", interview_id: "iv1", segment_index: 0, speaker: "A", timestamp_start: "00:00:01.000", timestamp_end: null, text: "one", block_id: null, section_tag: null },
      { id: "seg2", interview_id: "iv1", segment_index: 1, speaker: "A", timestamp_start: "00:00:02.000", timestamp_end: null, text: "two", block_id: null, section_tag: null },
      { id: "seg3", interview_id: "iv1", segment_index: 2, speaker: "A", timestamp_start: "00:00:03.000", timestamp_end: null, text: "three", block_id: null, section_tag: null },
      { id: "seg4", interview_id: "iv1", segment_index: 3, speaker: "A", timestamp_start: "00:00:04.000", timestamp_end: null, text: "four", block_id: null, section_tag: null },
    ];
    resetStore({ segments, selectedSegmentId: "seg1" });

    const s = useProjectStore.getState();
    s.setSelectedSegmentId("seg2");
    s.setSelectedSegmentId("seg3");
    s.setSelectedSegmentId("seg4");
    expect(vi.mocked(api.saveWorkspaceState)).not.toHaveBeenCalled();

    vi.advanceTimersByTime(350);
    expect(vi.mocked(api.saveWorkspaceState)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.saveWorkspaceState).mock.calls[0][0]).toMatchObject({
      selected_segment_id: "seg4",
    });
  });

  it("cancels the pending debounce when closeProject is called", async () => {
    vi.mocked(api.saveWorkspaceState).mockClear();
    resetStore({ segments: [], selectedSegmentId: "seg1" });

    useProjectStore.getState().setSelectedSegmentId("seg2");
    await useProjectStore.getState().closeProject();

    vi.advanceTimersByTime(500);
    expect(vi.mocked(api.saveWorkspaceState)).toHaveBeenCalledTimes(1);
  });
});

describe("B5 — interviewCodedCount cached on store", () => {
  it("is a number field on the store, not a method", () => {
    const s: unknown = useProjectStore.getState();
    expect(typeof (s as Record<string, unknown>).interviewCodedCount).toBe("number");
    expect((s as Record<string, unknown>).getInterviewCodedCount).toBeUndefined();
  });

  it("recomputes when selectInterview loads codedSegments", async () => {
    const segments = [
      { id: "seg1", interview_id: "iv1", segment_index: 0, speaker: "A", timestamp_start: "00:00:01.000", timestamp_end: null, text: "one", block_id: null, section_tag: null },
    ];
    const coded: CodedSegment[] = [
      { id: "cs1", interview_id: "iv1", segment_id: "seg1", code_ids: ["c1"], coder_name: "Alice", memo: null, char_start: null, char_end: null, quote_text: "x", block_id: null, timestamp_start: "00:00:01.000", participant_label: "P" },
      { id: "cs2", interview_id: "iv1", segment_id: "seg1", code_ids: ["c2"], coder_name: "Bob", memo: null, char_start: null, char_end: null, quote_text: "x", block_id: null, timestamp_start: "00:00:01.000", participant_label: "P" },
    ];
    vi.mocked(api.getSegments).mockResolvedValue(segments);
    vi.mocked(api.listCodedSegments).mockResolvedValue(coded);
    vi.mocked(api.listInterviews).mockResolvedValue([
      { id: "iv1", participant_label: "P1", interview_date: null, modality: null, diagnosis_notes: null, interviewers: [], hub_memo: null, audio_path: null, segment_count: 1, remote_segment_count: null },
    ]);

    await useProjectStore.getState().selectInterview("iv1");

    expect(useProjectStore.getState().interviewCodedCount).toBe(1);
    expect(computeInterviewCodedCount(coded)).toBe(1);
  });

  it("resets to 0 in closeProject", async () => {
    useProjectStore.setState({ interviewCodedCount: 5 });
    await useProjectStore.getState().closeProject();
    expect(useProjectStore.getState().interviewCodedCount).toBe(0);
  });
});

describe("B7 — selectAdjacentSegment decoupled from DOM", () => {
  it("does not call document.getElementById", () => {
    const getElementById = vi.fn();
    vi.stubGlobal("document", { getElementById });
    const segments = [
      { id: "seg1", interview_id: "iv1", segment_index: 0, speaker: "A", timestamp_start: "00:00:01.000", timestamp_end: null, text: "one", block_id: null, section_tag: null },
      { id: "seg2", interview_id: "iv1", segment_index: 1, speaker: "A", timestamp_start: "00:00:02.000", timestamp_end: null, text: "two", block_id: null, section_tag: null },
    ];
    resetStore({ segments, selectedSegmentId: "seg1" });

    useProjectStore.getState().selectAdjacentSegment("next");

    expect(getElementById).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
describe("identity — coding is blocked until someone says who they are", () => {
  it("refuses to apply codes with an unconfirmed coder and raises the prompt", async () => {
    resetStore({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg1",
      codes: [{ id: "c1", name: "test", color: "#8a6410", sort_order: 0, is_retired: false, usage_count: 0 }],
      selectedCodeIds: [],
      activeCoder: "",
      coderConfirmed: false,
    });

    await useProjectStore.getState().toggleCodeOnTarget("c1");

    expect(api.applyCodes).not.toHaveBeenCalled();
    const state = useProjectStore.getState();
    expect(state.toasts.some((t) => t.type === "error" || t.text.includes("identity"))).toBe(true);
    expect(state.showIdentityPrompt).toBe(true);
  });

  it("confirming a coder remembers them for this project", async () => {
    vi.mocked(api.getAppPreferences).mockResolvedValue({
      reopen_last_project: false,
      last_guide_section_id: null,
      panel_widths: null,
      coach_dismissed: false,
      merge_same_speaker: true,
      theme: "light",
      coder_identities: {},
      sync_url: null,
      sync_anon_key: null,
    });
    vi.mocked(api.setAppPreferences).mockImplementation(async (p) => p);
    resetStore({
      activeCoder: "",
      coderConfirmed: false,
      showIdentityPrompt: true,
    });

    await useProjectStore.getState().confirmCoder("Bina");

    expect(useProjectStore.getState().activeCoder).toBe("Bina");
    expect(useProjectStore.getState().coderConfirmed).toBe(true);
    expect(useProjectStore.getState().showIdentityPrompt).toBe(false);
    expect(api.setAppPreferences).toHaveBeenCalledWith(
      expect.objectContaining({
        coder_identities: { [baseProject.path]: "Bina" },
      }),
    );
  });

    it("adoptCoderName collapses the local name into the group name", () => {
    vi.mocked(api.getAppPreferences).mockResolvedValue({
      reopen_last_project: false,
      last_guide_section_id: null,
      panel_widths: null,
      coach_dismissed: false,
      merge_same_speaker: true,
      theme: "light",
      coder_identities: {},
      sync_url: null,
      sync_anon_key: null,
    });
    vi.mocked(api.setAppPreferences).mockImplementation(async (p) => p);
    resetStore({
      activeCoder: "Alice",
      coderConfirmed: true,
      showIdentityPrompt: false,
    });

    useProjectStore.getState().adoptCoderName("Ada Lovelace");

    expect(useProjectStore.getState().activeCoder).toBe("Ada Lovelace");
    expect(useProjectStore.getState().coderConfirmed).toBe(true);
    expect(useProjectStore.getState().showIdentityPrompt).toBe(false);
    expect(useProjectStore.getState().project?.coders).toEqual(["Ada Lovelace"]);
  });

});


describe("clicking a code applies it, rather than staging an intention", () => {
  function coded(over: Partial<CodedSegment> = {}): CodedSegment {
    return {
      id: "cs1",
      interview_id: "iv1",
      segment_id: "seg1",
      code_ids: ["c1"],
      coder_name: "Alice",
      memo: null,
      char_start: null,
      char_end: null,
      quote_text: "x",
      block_id: null,
      timestamp_start: "00:00:01.000",
      participant_label: "P",
      ...over,
    } as CodedSegment;
  }

  function ready(codedSegments: CodedSegment[] = []) {
    useProjectStore.setState({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg1",
      activeCoder: "Alice",
      coderConfirmed: true,
      codedSegments,
      pendingSelection: null,
      selectedCodeIds: codedSegments[0]?.code_ids ?? [],
    });
  }

  beforeEach(() => {
    vi.mocked(api.applyCodes).mockReset().mockResolvedValue({} as never);
    vi.mocked(api.deleteCodedSegment).mockReset().mockResolvedValue(undefined);
    vi.mocked(api.listCodedSegments).mockReset().mockResolvedValue([]);
  });

  it("writes immediately on the first click", async () => {
    ready();
    await useProjectStore.getState().toggleCodeOnTarget("c1");

    expect(api.mutateCodingEdge).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.mutateCodingEdge).mock.calls[0][0]).toMatchObject({
      segment_id: "seg1",
      code_id: "c1",
      present: true,
      coder_name: "Alice",
    });
  });

  it("adds to the set rather than replacing it", async () => {
    ready([coded({ code_ids: ["c1"] })]);
    await useProjectStore.getState().toggleCodeOnTarget("c2");

    expect(vi.mocked(api.mutateCodingEdge).mock.calls[0][0]).toMatchObject({
      code_id: "c2",
      present: true,
    });
    expect(useProjectStore.getState().codedSegments[0]?.code_ids).toEqual(["c1", "c2"]);
  });

  it("clicking an applied code removes it", async () => {
    ready([coded({ code_ids: ["c1", "c2"] })]);
    await useProjectStore.getState().toggleCodeOnTarget("c1");

    expect(vi.mocked(api.mutateCodingEdge).mock.calls[0][0]).toMatchObject({
      code_id: "c1",
      present: false,
    });
    expect(useProjectStore.getState().codedSegments[0]?.code_ids).toEqual(["c2"]);
  });

  it("removing the last code deletes the row instead of leaving an empty one", async () => {
    ready([coded({ code_ids: ["c1"] })]);
    await useProjectStore.getState().toggleCodeOnTarget("c1");

    expect(api.mutateCodingEdge).toHaveBeenCalledWith(expect.objectContaining({
      code_id: "c1",
      present: false,
    }));
    expect(useProjectStore.getState().codedSegments).toEqual([]);
  });

  it("keeps the note when a second code is added", async () => {
    ready([coded({ code_ids: ["c1"], memo: "worth revisiting" })]);
    await useProjectStore.getState().toggleCodeOnTarget("c2");

    expect(useProjectStore.getState().codedSegments[0]?.memo).toBe("worth revisiting");
  });

  it("targets the selected span, not the whole turn, when text is selected", async () => {
    ready([coded({ code_ids: ["c1"] })]);
    useProjectStore.setState({
      pendingSelection: { segmentId: "seg1", start: 4, end: 12, text: "rehearse" },
    });

    await useProjectStore.getState().toggleCodeOnTarget("c2");

    const call = vi.mocked(api.mutateCodingEdge).mock.calls[0][0];
    expect(call.char_start).toBe(4);
    expect(call.char_end).toBe(12);
    expect(call.code_id).toBe("c2");
    expect(call.present).toBe(true);
  });

  it("refuses when nobody has said who is coding", async () => {
    ready();
    useProjectStore.setState({ coderConfirmed: false });

    await useProjectStore.getState().toggleCodeOnTarget("c1");

    expect(api.mutateCodingEdge).not.toHaveBeenCalled();
    expect(useProjectStore.getState().showIdentityPrompt).toBe(true);
  });

  it("rolls the ticks back when the write fails", async () => {
    ready([coded({ code_ids: ["c1"] })]);
    vi.mocked(api.mutateCodingEdge).mockRejectedValueOnce(new Error("disk full"));

    await useProjectStore.getState().toggleCodeOnTarget("c2");

    expect(useProjectStore.getState().selectedCodeIds).toEqual(["c1"]);
    expect(useProjectStore.getState().toasts.some((t) => t.type === "error" && t.text.includes("disk full"))).toBe(true);
  });
});


describe("undo", () => {
  function coded(over: Partial<CodedSegment> = {}): CodedSegment {
    return {
      id: "cs1",
      interview_id: "iv1",
      segment_id: "seg1",
      code_ids: ["c1"],
      coder_name: "Alice",
      memo: null,
      char_start: null,
      char_end: null,
      quote_text: "x",
      block_id: null,
      timestamp_start: "00:00:01.000",
      participant_label: "P",
      ...over,
    } as CodedSegment;
  }

  beforeEach(() => {
    vi.mocked(api.applyCodes).mockReset().mockResolvedValue({} as never);
    vi.mocked(api.deleteCodedSegment).mockReset().mockResolvedValue(undefined);
    vi.mocked(api.listCodedSegments).mockReset().mockResolvedValue([]);
    useProjectStore.setState({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg1",
      activeCoder: "Alice",
      coderConfirmed: true,
      codedSegments: [],
      pendingSelection: null,
      selectedCodeIds: [],
      undoStack: [],
      redoStack: [],
      codes: [{ id: "c1", name: "masking" }] as never,
    });
  });

  it("undoing a freshly applied code removes it again", async () => {
    await useProjectStore.getState().toggleCodeOnTarget("c1");
    expect(api.mutateCodingEdge).toHaveBeenCalledTimes(1);

    // The row now exists, as the app would see after reloading.
    useProjectStore.setState({ codedSegments: [coded({ code_ids: ["c1"] })] });
    await useProjectStore.getState().undoLastCoding();

    const lastCall = vi.mocked(api.mutateCodingEdge).mock.calls[
      vi.mocked(api.mutateCodingEdge).mock.calls.length - 1
    ];
    expect(lastCall?.[0]).toMatchObject({
      segment_id: "seg1",
      code_id: "c1",
      present: false,
    });
  });

  it("undoing a removal restores the codes and the note", async () => {
    useProjectStore.setState({
      codedSegments: [coded({ code_ids: ["c1", "c2"], memo: "keep me" })],
    });
    await useProjectStore.getState().removeCodedSegment("cs1");
    await useProjectStore.getState().undoLastCoding();

    const calls = vi.mocked(api.mutateCodingEdge).mock.calls;
    expect(calls.slice(-2).map((call) => call[0].code_id)).toEqual(["c1", "c2"]);
    expect(calls.slice(-2).every((call) => call[0].present)).toBe(true);
    expect(api.patchCodingMemo).toHaveBeenCalledWith({
      coded_segment_id: expect.any(String),
      memo: "keep me",
    });
  });

  it("restores another coder's work under their name, not yours", async () => {
    useProjectStore.setState({
      codedSegments: [coded({ coder_name: "Sam", code_ids: ["c1"] })],
    });
    await useProjectStore.getState().removeCodedSegment("cs1");
    await useProjectStore.getState().undoLastCoding();

    const calls = vi.mocked(api.mutateCodingEdge).mock.calls;
    expect(calls[calls.length - 1][0].coder_name).toBe("Sam");
  });

  it("says so plainly when there is nothing to undo", async () => {
    await useProjectStore.getState().undoLastCoding();
    expect(useProjectStore.getState().toasts.some((t) => /nothing to undo/i.test(t.text))).toBe(true);
    expect(api.mutateCodingEdge).not.toHaveBeenCalled();
  });

  it("pops a failed undo instead of leaving it to fail again", async () => {
    await useProjectStore.getState().toggleCodeOnTarget("c1");
    useProjectStore.setState({ codedSegments: [coded({ code_ids: ["c1"] })] });
    vi.mocked(api.mutateCodingEdge).mockRejectedValueOnce(new Error("locked"));

    await useProjectStore.getState().undoLastCoding();

    expect(useProjectStore.getState().toasts.some((t) => t.type === "error")).toBe(true);
    expect(useProjectStore.getState().undoStack).toHaveLength(0);
  });

  it("keeps the stack bounded", async () => {
    for (let i = 0; i < 40; i++) {
      useProjectStore.getState().pushUndo(`step ${i}`, async () => {}, async () => {});
    }
    expect(useProjectStore.getState().undoStack.length).toBeLessThanOrEqual(25);
    // And it is the *recent* end that survives.
    const stack = useProjectStore.getState().undoStack;
    expect(stack[stack.length - 1].label).toBe("step 39");
  });

  it("undoing acts on the coded row, not the passage selection moved to", async () => {
    // Code seg1, then click over to seg2 — which carries its own coding —
    // and press undo. The row removed must be seg1's, or undo is a
    // wrong-row delete.
    await useProjectStore.getState().toggleCodeOnTarget("c1");
    useProjectStore.setState({
      codedSegments: [
        coded({ id: "cs1", segment_id: "seg1", code_ids: ["c1"] }),
        coded({ id: "cs2", segment_id: "seg2", code_ids: ["c1"] }),
      ],
      selectedSegmentId: "seg2",
    });

    await useProjectStore.getState().undoLastCoding();

    const lastCall = vi.mocked(api.mutateCodingEdge).mock.calls[
      vi.mocked(api.mutateCodingEdge).mock.calls.length - 1
    ];
    expect(lastCall?.[0]).toMatchObject({
      segment_id: "seg1",
      code_id: "c1",
      present: false,
    });
  });
});

describe("redo", () => {
  function coded(over: Partial<CodedSegment> = {}): CodedSegment {
    return {
      id: "cs1",
      interview_id: "iv1",
      segment_id: "seg1",
      code_ids: ["c1"],
      coder_name: "Alice",
      memo: null,
      char_start: null,
      char_end: null,
      quote_text: "x",
      block_id: null,
      timestamp_start: "00:00:01.000",
      participant_label: "P",
      ...over,
    } as CodedSegment;
  }

  beforeEach(() => {
    vi.mocked(api.applyCodes).mockReset().mockResolvedValue({} as never);
    vi.mocked(api.deleteCodedSegment).mockReset().mockResolvedValue(undefined);
    vi.mocked(api.listCodedSegments).mockReset().mockResolvedValue([]);
    useProjectStore.setState({
      activeInterviewId: "iv1",
      selectedSegmentId: "seg1",
      activeCoder: "Alice",
      coderConfirmed: true,
      codedSegments: [],
      pendingSelection: null,
      selectedCodeIds: [],
      undoStack: [],
      redoStack: [],
      codes: [{ id: "c1", name: "masking" }] as never,
    });
  });

  it("redoing an undone application puts the code back", async () => {
    await useProjectStore.getState().toggleCodeOnTarget("c1");
    useProjectStore.setState({ codedSegments: [coded({ code_ids: ["c1"] })] });
    await useProjectStore.getState().undoLastCoding();
    const lastCall = vi.mocked(api.mutateCodingEdge).mock.calls[
      vi.mocked(api.mutateCodingEdge).mock.calls.length - 1
    ];
    expect(lastCall?.[0]).toMatchObject({
      code_id: "c1",
      present: false,
    });

    // After the undo, the row is gone — as a reload would see it.
    useProjectStore.setState({ codedSegments: [] });
    vi.mocked(api.mutateCodingEdge).mockClear();
    await useProjectStore.getState().redoLastCoding();

    expect(api.mutateCodingEdge).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.mutateCodingEdge).mock.calls[0][0]).toMatchObject({
      code_id: "c1",
      present: true,
    });
  });

  it("redoing an undone removal removes it again", async () => {
    useProjectStore.setState({
      codedSegments: [coded({ code_ids: ["c1"] })],
    });
    await useProjectStore.getState().removeCodedSegment("cs1");
    await useProjectStore.getState().undoLastCoding();

    // The row is back, as the app would see after the restore.
    useProjectStore.setState({ codedSegments: [coded({ code_ids: ["c1"] })] });
    vi.mocked(api.mutateCodingEdge).mockClear();
    await useProjectStore.getState().redoLastCoding();

    expect(vi.mocked(api.mutateCodingEdge).mock.calls[0][0]).toMatchObject({
      code_id: "c1",
      present: false,
    });
  });

  it("a redo lands back on the undo stack, so ⌘Z and ⇧⌘Z walk both ways", async () => {
    await useProjectStore.getState().toggleCodeOnTarget("c1");
    useProjectStore.setState({ codedSegments: [coded({ code_ids: ["c1"] })] });
    await useProjectStore.getState().undoLastCoding();
    await useProjectStore.getState().redoLastCoding();

    expect(useProjectStore.getState().undoStack).toHaveLength(1);
    expect(useProjectStore.getState().redoStack).toHaveLength(0);
  });

  it("a fresh action clears the redo stack", async () => {
    await useProjectStore.getState().toggleCodeOnTarget("c1");
    useProjectStore.setState({ codedSegments: [coded({ code_ids: ["c1"] })] });
    await useProjectStore.getState().undoLastCoding();
    expect(useProjectStore.getState().redoStack).toHaveLength(1);

    await useProjectStore.getState().toggleCodeOnTarget("c1");
    expect(useProjectStore.getState().redoStack).toHaveLength(0);

    await useProjectStore.getState().redoLastCoding();
    expect(useProjectStore.getState().toasts.some((t) => /nothing to redo/i.test(t.text))).toBe(true);
  });

  it("a failed undo is not offered as a redo", async () => {
    await useProjectStore.getState().toggleCodeOnTarget("c1");
    useProjectStore.setState({ codedSegments: [coded({ code_ids: ["c1"] })] });
    vi.mocked(api.mutateCodingEdge).mockRejectedValueOnce(new Error("locked"));

    await useProjectStore.getState().undoLastCoding();

    expect(useProjectStore.getState().redoStack).toHaveLength(0);
  });
});

describe("v0.20 — toasts, targeted code removal, note clearing, and recent codes", () => {
  function makeCoded(over: Partial<CodedSegment> = {}): CodedSegment {
    return {
      id: "cs1",
      interview_id: "iv1",
      segment_id: "seg1",
      code_ids: ["c1"],
      coder_name: "Alice",
      memo: null,
      char_start: null,
      char_end: null,
      quote_text: "x",
      block_id: null,
      timestamp_start: "00:00:01.000",
      participant_label: "P",
      ...over,
    } as CodedSegment;
  }

  it("showStatus limits toasts to 3 with FIFO eviction", () => {
    const s = useProjectStore.getState();
    s.clearToasts();

    s.showStatus("Message 1", "info");
    s.showStatus("Message 2", "info");
    s.showStatus("Message 3", "info");
    expect(useProjectStore.getState().toasts).toHaveLength(3);
    expect(useProjectStore.getState().toasts.map((t) => t.text)).toEqual([
      "Message 1",
      "Message 2",
      "Message 3",
    ]);

    s.showStatus("Message 4", "info");
    expect(useProjectStore.getState().toasts).toHaveLength(3);
    expect(useProjectStore.getState().toasts.map((t) => t.text)).toEqual([
      "Message 2",
      "Message 3",
      "Message 4",
    ]);
  });

  it("removeCodeFromCoding updates multi-code row and attaches Undo toast", async () => {
    const coding = makeCoded({ id: "cs1", code_ids: ["c1", "c2"] });
    useProjectStore.setState({
      codedSegments: [coding],
      codes: [
        { id: "c1", name: "Masking", color: "#8a6410", usage_count: 1 } as never,
        { id: "c2", name: "Fatigue", color: "#1f7a5e", usage_count: 1 } as never,
      ],
    });
    await useProjectStore.getState().removeCodeFromCoding("cs1", "c1");

    expect(api.mutateCodingEdge).toHaveBeenCalledWith(
      expect.objectContaining({
        code_id: "c1",
        present: false,
      }),
    );
    const toasts = useProjectStore.getState().toasts;
    expect(toasts.some((t) => t.text.includes('Removed "Masking"'))).toBe(true);
    expect(useProjectStore.getState().undoStack).toHaveLength(1);
  });

  it("clearMemoForCoding saves empty memo and registers undo", async () => {
    const coding = makeCoded({ id: "cs1", memo: "My note on this passage" });
    useProjectStore.setState({
      codedSegments: [coding],
    });
    vi.mocked(api.patchCodingMemo).mockResolvedValueOnce({
      ...coding,
      memo: "",
    });

    await useProjectStore.getState().clearMemoForCoding("cs1");

    expect(api.patchCodingMemo).toHaveBeenCalledWith({
      coded_segment_id: "cs1",
      memo: undefined,
    });
    const toasts = useProjectStore.getState().toasts;
    expect(toasts.some((t) => t.text.includes("Note removed"))).toBe(true);
    expect(useProjectStore.getState().undoStack).toHaveLength(1);
  });
});

describe("v0.20.1 regressions — red tests", () => {
  it("openNoteForCoding targets an explicit coding ID and is independent of transcript selection", () => {
    resetStore({
      noteEditorCodingId: null,
      selectedSegmentId: "seg1",
    });

    const store = useProjectStore.getState() as any;
    expect(typeof store.openNoteForCoding).toBe("function");
    store.openNoteForCoding("cs-42");
    expect(useProjectStore.getState().noteEditorCodingId).toBe("cs-42");

    // Changing selected segment does NOT clobber noteEditorCodingId
    useProjectStore.getState().setSelectedSegmentId("seg2");
    expect(useProjectStore.getState().noteEditorCodingId).toBe("cs-42");
  });

  it("saveMemoForCoding throws on API error instead of swallowing it", async () => {
    const coding = {
      id: "cs-fail",
      interview_id: "iv1",
      segment_id: "seg1",
      code_ids: ["c1"],
      coder_name: "Alice",
      memo: "Initial memo",
      char_start: null,
      char_end: null,
      quote_text: "quote",
      block_id: null,
      timestamp_start: "00:00:01",
      participant_label: "P",
    };
    resetStore({
      codedSegments: [coding],
    });
    vi.mocked(api.patchCodingMemo).mockRejectedValueOnce(new Error("Disk I/O failure"));

    const store = useProjectStore.getState() as any;
    await expect(store.saveMemoForCoding("cs-fail", "New draft memo")).rejects.toThrow("Disk I/O failure");
  });

  it("updateCode forwards parent_id to api.updateCode", async () => {
    vi.mocked(api.updateCode).mockClear();
    await useProjectStore.getState().updateCode({
      id: "code-child",
      name: "Masking Detail",
      definition: "A sub-code",
      color: "#8a6410",
      parent_id: "code-parent",
    });

    expect(api.updateCode).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.updateCode).mock.calls[0][0]).toEqual({
      id: "code-child",
      name: "Masking Detail",
      definition: "A sub-code",
      inclusion_criteria: null,
      exclusion_criteria: null,
      example: null,
      parent_id: "code-parent",
      color: "#8a6410",
    });
  });

  it("updateCode preserves inclusion_criteria, exclusion_criteria, and example", async () => {
    vi.mocked(api.updateCode).mockClear();
    await useProjectStore.getState().updateCode({
      id: "code-1",
      name: "Masking",
      definition: "General masking",
      inclusion_criteria: "Eye contact compensation",
      exclusion_criteria: "General shyness",
      example: "Practicing facial expressions",
      parent_id: null,
      color: "#8a6410",
    });

    expect(api.updateCode).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.updateCode).mock.calls[0][0]).toEqual({
      id: "code-1",
      name: "Masking",
      definition: "General masking",
      inclusion_criteria: "Eye contact compensation",
      exclusion_criteria: "General shyness",
      example: "Practicing facial expressions",
      parent_id: null,
      color: "#8a6410",
    });
  });

  it("reparentCode updates parent_id and pushes an undo step that restores previous parent", async () => {
    const parent = {
      id: "parent-1",
      name: "Parent",
      definition: null,
      inclusion_criteria: null,
      exclusion_criteria: null,
      example: null,
      parent_id: null,
      color: "#8a6410",
      sort_order: 0,
      is_retired: false,
      usage_count: 0,
    };
    const child = {
      id: "child-1",
      name: "Child",
      definition: "Sub-code",
      inclusion_criteria: null,
      exclusion_criteria: null,
      example: null,
      parent_id: null,
      color: "#8a6410",
      sort_order: 1,
      is_retired: false,
      usage_count: 0,
    };

    resetStore({
      codes: [parent, child],
      undoStack: [],
    });

    vi.mocked(api.updateCode).mockClear();
    await useProjectStore.getState().reparentCode("child-1", "parent-1");

    expect(api.updateCode).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.updateCode).mock.calls[0][0].parent_id).toBe("parent-1");

    const undoStack = useProjectStore.getState().undoStack;
    expect(undoStack).toHaveLength(1);
    expect(undoStack[0].label).toContain('Nest "Child" under "Parent"');

    // Trigger undo:
    vi.mocked(api.updateCode).mockClear();
    await useProjectStore.getState().undoLastCoding();

    expect(api.updateCode).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.updateCode).mock.calls[0][0].parent_id).toBeNull();
  });
});
