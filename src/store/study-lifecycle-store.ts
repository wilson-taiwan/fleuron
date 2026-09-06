import { create } from "zustand";
import {
  studyLifecycle,
  type InFlightOperation,
  type LifecycleResult,
} from "../lib/study-lifecycle";
import type { StudyProblem } from "../lib/study-problem";

interface StudyLifecycleState {
  activeOp: InFlightOperation | null;
  lastResult: LifecycleResult | null;
  lastProblem: StudyProblem | null;
  openStudy: (path: string) => Promise<LifecycleResult>;
  removeStudy: (params: {
    mode: "delete_solo" | "detach" | "leave" | "delete_group";
    path?: string;
    projectId?: string;
    title: string;
    alsoDeleteFolder?: boolean;
  }) => Promise<LifecycleResult>;
  saveLocalCopy: (sourcePath: string, destinationDir: string) => Promise<LifecycleResult>;
  restoreStudyBackup: (
    archivePath: string,
    parentDir?: string,
    targetTitle?: string
  ) => Promise<LifecycleResult<string>>;
  clearProblem: () => void;
}

export const useStudyLifecycleStore = create<StudyLifecycleState>((set) => ({
  activeOp: null,
  lastResult: null,
  lastProblem: null,

  openStudy: async (path: string) => {
    set({ activeOp: studyLifecycle.getActiveOperation() });
    const result = await studyLifecycle.openStudy(path);
    set({
      activeOp: studyLifecycle.getActiveOperation(),
      lastResult: result,
      lastProblem: result.problem || null,
    });
    return result;
  },

  removeStudy: async (params) => {
    set({ activeOp: studyLifecycle.getActiveOperation() });
    const result = await studyLifecycle.removeStudy(params);
    set({
      activeOp: studyLifecycle.getActiveOperation(),
      lastResult: result,
      lastProblem: result.problem || null,
    });
    return result;
  },

  saveLocalCopy: async (sourcePath: string, destinationDir: string) => {
    const result = await studyLifecycle.saveLocalCopy(sourcePath, destinationDir);
    set({
      lastResult: result,
      lastProblem: result.problem || null,
    });
    return result;
  },

  restoreStudyBackup: async (archivePath: string, parentDir?: string, targetTitle?: string) => {
    const result = await studyLifecycle.restoreStudyBackup(archivePath, parentDir, targetTitle);
    set({
      lastResult: result,
      lastProblem: result.problem || null,
    });
    return result;
  },

  clearProblem: () => set({ lastProblem: null }),
}));
