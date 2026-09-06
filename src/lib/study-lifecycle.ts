import { api } from "./api";
import { useProjectStore } from "../store/project-store";
import { useNoteDepartureStore } from "../store/note-departure-store";
import {
  type StudyLifecycleOperation,
  type StudyLifecycleStage,
  type StudyProblem,
  classifyStudyProblem,
} from "./study-problem";

export type LifecycleResultStatus =
  | "completed"
  | "cancelled"
  | "busy"
  | "failed"
  | "partial"
  | "remote-outcome-unknown";

export interface LifecycleResult<T = any> {
  status: LifecycleResultStatus;
  operationId: string;
  operation: StudyLifecycleOperation;
  target?: string;
  completedStage?: StudyLifecycleStage;
  data?: T;
  problem?: StudyProblem;
}

export interface InFlightOperation {
  operationId: string;
  operation: StudyLifecycleOperation;
  target?: string;
  stage: StudyLifecycleStage;
  startedAt: number;
}

let activeOperation: InFlightOperation | null = null;
let activePromise: Promise<LifecycleResult> | null = null;

function mintOperationId(): string {
  return `op-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

export const studyLifecycle = {
  getActiveOperation(): InFlightOperation | null {
    return activeOperation;
  },

  isBusy(): boolean {
    return activeOperation !== null;
  },

  async openStudy(path: string): Promise<LifecycleResult> {
    if (activeOperation) {
      if (activeOperation.operation === "open" && activeOperation.target === path) {
        return (await activePromise) || {
          status: "busy",
          operationId: activeOperation.operationId,
          operation: "open",
          target: path,
        };
      }
      return {
        status: "busy",
        operationId: mintOperationId(),
        operation: "open",
        target: path,
        problem: {
          code: "unexpected_failure",
          operation: "open",
          target: path,
          message: "Another study operation is currently in progress.",
          nextActions: ["cancel"],
          isRetryable: true,
        },
      };
    }

    const opId = mintOperationId();
    activeOperation = {
      operationId: opId,
      operation: "open",
      target: path,
      stage: "preflight",
      startedAt: Date.now(),
    };

    activePromise = (async () => {
      try {
        // 1. Guard against dirty note departures if a study is currently open
        const currentStudy = useProjectStore.getState().project;
        if (currentStudy && currentStudy.path !== path) {
          const departure = await useNoteDepartureStore.getState().requestDeparture("open-study");
          if (!departure.proceed) {
            return {
              status: "cancelled" as const,
              operationId: opId,
              operation: "open" as const,
              target: path,
            };
          }
        }

        activeOperation!.stage = "checking";
        await useProjectStore.getState().openProject(path);

        const openErr = useProjectStore.getState().error;
        if (openErr) {
          throw new Error(openErr);
        }

        activeOperation!.stage = "complete";
        return {
          status: "completed" as const,
          operationId: opId,
          operation: "open" as const,
          target: path,
          completedStage: "complete" as const,
        };
      } catch (err) {
        const problem = classifyStudyProblem(err, "open", {
          target: path,
          completedStage: activeOperation?.stage,
        });
        return {
          status: "failed" as const,
          operationId: opId,
          operation: "open" as const,
          target: path,
          completedStage: activeOperation?.stage,
          problem,
        };
      } finally {
        activeOperation = null;
        activePromise = null;
      }
    })();

    return activePromise;
  },

  async removeStudy(params: {
    mode: "delete_solo" | "detach" | "leave" | "delete_group";
    path?: string;
    projectId?: string;
    title: string;
    alsoDeleteFolder?: boolean;
  }): Promise<LifecycleResult> {
    if (activeOperation) {
      return {
        status: "busy",
        operationId: mintOperationId(),
        operation: params.mode === "delete_solo" ? "delete" : params.mode === "leave" ? "leave" : "detach",
        target: params.path || params.projectId,
      };
    }

    const opType: StudyLifecycleOperation =
      params.mode === "delete_solo" ? "delete" : params.mode === "leave" ? "leave" : "detach";
    const opId = mintOperationId();
    activeOperation = {
      operationId: opId,
      operation: opType,
      target: params.path || params.projectId,
      stage: "preflight",
      startedAt: Date.now(),
    };

    activePromise = (async () => {
      try {
        // If the study being removed is currently open, guard notes and close it
        const currentPath = useProjectStore.getState().project?.path;
        const isOpen = params.path && currentPath && currentPath === params.path;
        if (isOpen) {
          const departure = await useNoteDepartureStore.getState().requestDeparture("close-study");
          if (!departure.proceed) {
            return {
              status: "cancelled" as const,
              operationId: opId,
              operation: opType,
              target: params.path || params.projectId,
            };
          }
          await useProjectStore.getState().closeProject();
        }

        activeOperation!.stage = "handles_closed";

        if (params.mode === "delete_solo") {
          if (params.path) {
            activeOperation!.stage = "folder_trashed";
            await api.deleteProjectFolder(params.path);
            await api.removeRecentProject(params.path);
          }
        } else if (params.mode === "detach") {
          activeOperation!.stage = "local_unbound";
          await api.syncDetachLocal(params.projectId);
        } else if (params.mode === "leave") {
          activeOperation!.stage = "remote_confirmed";
          await api.syncLeaveGroup(params.projectId);
          activeOperation!.stage = "local_unbound";
          if (params.alsoDeleteFolder && params.path) {
            activeOperation!.stage = "folder_trashed";
            await api.deleteProjectFolder(params.path);
            await api.removeRecentProject(params.path);
          }
        } else if (params.mode === "delete_group") {
          activeOperation!.stage = "remote_confirmed";
          await api.syncDeleteGroup(params.title, params.projectId);
          if (params.alsoDeleteFolder && params.path) {
            activeOperation!.stage = "folder_trashed";
            await api.deleteProjectFolder(params.path);
            await api.removeRecentProject(params.path);
          }
        }

        activeOperation!.stage = "home_reconciled";
        return {
          status: "completed" as const,
          operationId: opId,
          operation: opType,
          target: params.path || params.projectId,
          completedStage: "complete" as const,
        };
      } catch (err) {
        const completedStage = activeOperation?.stage || "preflight";
        const problem = classifyStudyProblem(err, opType, {
          target: params.path || params.projectId,
          studyTitle: params.title,
          completedStage,
        });
        const isRemoteLost = problem.code === "response_lost";
        return {
          status: isRemoteLost ? ("remote-outcome-unknown" as const) : ("failed" as const),
          operationId: opId,
          operation: opType,
          target: params.path || params.projectId,
          completedStage,
          problem,
        };
      } finally {
        activeOperation = null;
        activePromise = null;
      }
    })();

    return activePromise;
  },

  async saveLocalCopy(sourcePath: string, destinationDir: string): Promise<LifecycleResult> {
    const opId = mintOperationId();
    try {
      const info = await api.saveLocalCopy(sourcePath, destinationDir, true);
      return {
        status: "completed",
        operationId: opId,
        operation: "preserve" as any,
        target: sourcePath,
        data: info,
      };
    } catch (err) {
      const problem = classifyStudyProblem(err, "preserve" as any, { target: sourcePath });
      return {
        status: "failed",
        operationId: opId,
        operation: "preserve" as any,
        target: sourcePath,
        problem,
      };
    }
  },

  async restoreStudyBackup(archivePath: string, parentDir?: string, targetTitle?: string): Promise<LifecycleResult<string>> {
    const opId = mintOperationId();
    try {
      const newPath = await api.restoreStudyBackup(archivePath, parentDir, targetTitle);
      return {
        status: "completed",
        operationId: opId,
        operation: "restore",
        target: newPath,
        data: newPath,
      };
    } catch (err) {
      const problem = classifyStudyProblem(err, "restore", { target: archivePath });
      return {
        status: "failed",
        operationId: opId,
        operation: "restore",
        target: archivePath,
        problem,
      };
    }
  },
};

export interface RemovalTargetInfo {
  title: string;
  path?: string;
  projectId?: string;
  isBound: boolean;
  isAdmin?: boolean;
  isRemoteOnly?: boolean;
  members?: string[];
}

export type RemovalMode = "detach" | "leave" | "delete_group" | "delete_solo";

export function determineAvailableRemovalModes(target: RemovalTargetInfo): RemovalMode[] {
  if (!target.isBound) {
    return ["delete_solo"];
  }
  const isSoleMember = target.members ? target.members.length <= 1 : false;
  if (target.isRemoteOnly) {
    const modes: RemovalMode[] = [];
    if (!isSoleMember) {
      modes.push("leave");
    }
    if (target.isAdmin) {
      modes.push("delete_group");
    }
    return modes;
  }
  const modes: RemovalMode[] = ["detach"];
  if (!isSoleMember) {
    modes.push("leave");
  }
  if (target.isAdmin) {
    modes.push("delete_group");
  }
  return modes;
}

export function getDefaultRemovalMode(target: RemovalTargetInfo, initialMode?: RemovalMode): RemovalMode {
  const isSoleMember = target.members ? target.members.length <= 1 : false;
  let defaultMode: RemovalMode =
    initialMode ??
    (!target.isBound
      ? "delete_solo"
      : target.isRemoteOnly
        ? isSoleMember && target.isAdmin
          ? "delete_group"
          : "leave"
        : "detach");

  if (isSoleMember && defaultMode === "leave") {
    if (!target.isRemoteOnly) {
      defaultMode = "detach";
    } else if (target.isAdmin) {
      defaultMode = "delete_group";
    }
  }
  return defaultMode;
}
