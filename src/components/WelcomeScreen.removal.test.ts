import { describe, expect, it, vi } from "vitest";
import {
  determineAvailableRemovalModes,
  getDefaultRemovalMode,
  studyLifecycle,
  type RemovalTargetInfo,
} from "../lib/study-lifecycle";

vi.mock("../lib/api", () => ({
  api: {
    deleteProjectFolder: vi.fn().mockResolvedValue(undefined),
    removeRecentProject: vi.fn().mockResolvedValue(undefined),
    syncDetachLocal: vi.fn().mockResolvedValue(undefined),
    syncLeaveGroup: vi.fn().mockResolvedValue(undefined),
    syncDeleteGroup: vi.fn().mockResolvedValue(undefined),
    projectDeletionSummary: vi.fn().mockResolvedValue({
      interviews: 1,
      coded_segments: 5,
      memos: 2,
    }),
  },
}));

describe("study-removal state machine", () => {
  it("determines available modes for solo unbound study", () => {
    const solo: RemovalTargetInfo = {
      title: "Solo Study",
      path: "/Users/test/Solo",
      isBound: false,
      isAdmin: false,
    };
    expect(determineAvailableRemovalModes(solo)).toEqual(["delete_solo"]);
    expect(getDefaultRemovalMode(solo)).toBe("delete_solo");
  });

  it("determines available modes for bound coder study (multi-member)", () => {
    const boundCoder: RemovalTargetInfo = {
      title: "Shared Project",
      path: "/Users/test/Shared",
      projectId: "proj-123",
      isBound: true,
      isAdmin: false,
      members: ["user-1", "user-2"],
    };
    expect(determineAvailableRemovalModes(boundCoder)).toEqual(["detach", "leave"]);
    expect(getDefaultRemovalMode(boundCoder)).toBe("detach");
  });

  it("determines available modes for bound admin study (multi-member)", () => {
    const boundAdmin: RemovalTargetInfo = {
      title: "Shared Project",
      path: "/Users/test/Shared",
      projectId: "proj-123",
      isBound: true,
      isAdmin: true,
      members: ["user-1", "user-2"],
    };
    expect(determineAvailableRemovalModes(boundAdmin)).toEqual(["detach", "leave", "delete_group"]);
    expect(getDefaultRemovalMode(boundAdmin)).toBe("detach");
  });

  it("blocks leave when user is sole member (last member) with local folder", () => {
    const soleMemberLocal: RemovalTargetInfo = {
      title: "Solo Shared Project",
      path: "/Users/test/SoloShared",
      projectId: "proj-123",
      isBound: true,
      isAdmin: true,
      members: ["user-1"],
    };
    // Sole member cannot leave (no orphan studies allowed); only detach or delete_group
    expect(determineAvailableRemovalModes(soleMemberLocal)).toEqual(["detach", "delete_group"]);
    expect(getDefaultRemovalMode(soleMemberLocal)).toBe("detach");
  });

  it("determines available modes for remote-only study (no local folder, multi-member)", () => {
    const remoteOnly: RemovalTargetInfo = {
      title: "Remote Group",
      projectId: "proj-456",
      isBound: true,
      isAdmin: false,
      isRemoteOnly: true,
      members: ["user-1", "user-2"],
    };
    expect(determineAvailableRemovalModes(remoteOnly)).toEqual(["leave"]);
    expect(getDefaultRemovalMode(remoteOnly)).toBe("leave");
  });

  it("blocks leave for remote-only sole member admin study", () => {
    const remoteSoleAdmin: RemovalTargetInfo = {
      title: "Remote Group",
      projectId: "proj-456",
      isBound: true,
      isAdmin: true,
      isRemoteOnly: true,
      members: ["user-1"],
    };
    // Cannot leave as sole member; must delete_group
    expect(determineAvailableRemovalModes(remoteSoleAdmin)).toEqual(["delete_group"]);
    expect(getDefaultRemovalMode(remoteSoleAdmin)).toBe("delete_group");
  });

  it("executes studyLifecycle.removeStudy for solo study removal", async () => {
    const result = await studyLifecycle.removeStudy({
      mode: "delete_solo",
      path: "/fake/projects/solo-to-delete",
      title: "Solo To Delete",
    });
    expect(result.status).toBe("completed");
    expect(result.operation).toBe("delete");
    expect(result.completedStage).toBe("complete");
  });

  it("executes studyLifecycle.removeStudy for detach mode", async () => {
    const result = await studyLifecycle.removeStudy({
      mode: "detach",
      projectId: "proj-detach-test",
      title: "Detach Test",
    });
    expect(result.status).toBe("completed");
    expect(result.operation).toBe("detach");
    expect(result.completedStage).toBe("complete");
  });
});
