import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

vi.mock("../lib/api", () => ({
  api: {
    getUpdateStatus: vi.fn(),
    updateCheck: vi.fn(),
    updateDownload: vi.fn(),
    updateCancelDownload: vi.fn(),
    updateInstall: vi.fn(),
    // Draft-preflight surface used by installUpdate.
    noteRecoveryStatus: vi.fn().mockResolvedValue({ available: true, error: null }),
    listNoteDrafts: vi.fn().mockResolvedValue([]),
    approveUpdateDeparture: vi.fn().mockResolvedValue({
      token: "test-approval",
      project_key: null,
      epoch: null,
      draft_write_seq: 0,
    }),
  },
}));

import { api } from "../lib/api";
import { useUpdateStore } from "./update-store";
import { useNoteDepartureStore } from "./note-departure-store";
import { useNoteDraftStore, type NoteDraftEntry } from "./note-draft-store";

const availableStatus = {
  phase: "available" as const,
  currentVersion: "0.26.1",
  targetVersion: "0.27.0",
  downloadedBytes: 0,
  totalBytes: null,
  lastCheckedAt: "2026-08-23T00:00:00Z",
  syncPreflightOutcome: null,
  failure: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  useUpdateStore.setState({
    status: availableStatus,
    update: { version: "0.27.0" },
    checking: false,
    downloading: false,
    installing: false,
    listenerStarted: false,
  });
  vi.mocked(api.getUpdateStatus).mockResolvedValue(availableStatus);
});

describe("v0.27 updater ordering regression", () => {
  it("does not invoke install preparation when verified download fails", async () => {
    vi.mocked(api.updateDownload).mockRejectedValue(new Error("synthetic download failure"));

    await useUpdateStore.getState().downloadUpdate();

    expect(api.updateDownload).toHaveBeenCalledTimes(1);
    expect(api.updateInstall).not.toHaveBeenCalled();
    expect(api.getUpdateStatus).toHaveBeenCalledTimes(1);
  });

  it("does not let React relaunch after native install begins", async () => {
    vi.mocked(api.updateInstall).mockResolvedValue({
      ...availableStatus,
      phase: "preparing",
    });

    await useUpdateStore.getState().installUpdate();

    expect(api.updateInstall).toHaveBeenCalledTimes(1);
    expect(useUpdateStore.getState().installing).toBe(true);
  });

  it("moves the primary action through check, download, then native install", async () => {
    vi.mocked(api.updateDownload).mockResolvedValue({
      ...availableStatus,
      phase: "readyToInstall",
    });
    await useUpdateStore.getState().runPrimaryAction();
    expect(api.updateDownload).toHaveBeenCalledTimes(1);

    await useUpdateStore.getState().runPrimaryAction();
    expect(api.updateInstall).toHaveBeenCalledTimes(1);
  });

  it("runs the draft preflight before install and passes its approval token", async () => {
    vi.mocked(api.updateInstall).mockResolvedValue({
      ...availableStatus,
      phase: "preparing",
    });

    await useUpdateStore.getState().installUpdate();

    expect(api.approveUpdateDeparture).toHaveBeenCalledTimes(1);
    expect(api.updateInstall).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.updateInstall).mock.calls[0][0]).toBe("test-approval");
  });

  it("cancelling the draft preflight installs nothing and mutates nothing", async () => {
    // One attached dirty draft forces the typed choice prompt.
    const key = "proj-1::coding::coding-1";
    const dirty: NoteDraftEntry = {
      key,
      projectKey: "proj-1",
      kind: "coding",
      targetId: "coding-1",
      interviewId: "iv-1",
      epoch: "epoch-1",
      draftId: "draft-1",
      revision: 1,
      ackedRevision: 1,
      baseText: "",
      draftText: "unsaved",
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
    useNoteDraftStore.setState({
      workspace: { projectKey: "proj-1", epoch: "epoch-1" },
      entries: { [key]: dirty },
      frozen: false,
    });

    const install = useUpdateStore.getState().installUpdate();
    // The prompt is up; cancel it like a user would.
    await vi.waitFor(() => {
      expect(useNoteDepartureStore.getState().prompt).not.toBeNull();
    });
    useNoteDepartureStore.getState().choose("cancelled");
    await install;

    expect(api.approveUpdateDeparture).not.toHaveBeenCalled();
    expect(api.updateInstall).not.toHaveBeenCalled();
    expect(useNoteDraftStore.getState().frozen).toBe(false);
    useNoteDraftStore.getState().clearWorkspace();
  });
});
