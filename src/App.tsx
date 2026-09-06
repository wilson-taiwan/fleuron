import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useProjectStore } from "./store/project-store";
import { WelcomeScreen } from "./components/WelcomeScreen";
import { WorkspaceLayout } from "./components/WorkspaceLayout";
import { UserGuidePanel } from "./components/UserGuidePanel";
import { AboutModal } from "./components/AboutModal";
import { SetupWizard } from "./components/setup/SetupWizard";
import { SyncSheet } from "./components/SyncSheet";
import { CoderIdentityModal } from "./components/CoderIdentityModal";
import { JoinStudyModal } from "./components/JoinStudyModal";
import { SettingsSheet } from "./components/SettingsSheet";
import { TrustCenterPanel } from "./components/TrustCenterPanel";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { OpeningOverlay } from "./components/OpeningOverlay";
import { UpdatePreparationOverlay } from "./components/UpdateAction";
import { SelftestRunner } from "./selftest/runner";
import { ToastStack } from "./components/ToastStack";
import { NoteRecoveryPanel } from "./components/NoteRecoveryPanel";
import { api } from "./lib/api";
import { useAppInit, useGlobalKeyboardShortcuts } from "./hooks/useAppInit";
import { useMenuEvents } from "./hooks/useMenuEvents";
import { useNativeDeparture } from "./hooks/useNativeDeparture";
import { NoteDepartureDialog } from "./components/NoteDepartureDialog";

function App() {
  const project = useProjectStore((s) => s.project);
  const loading = useProjectStore((s) => s.loading);
  const openingPath = useProjectStore((s) => s.openingPath);
  const [isSelftest, setIsSelftest] = useState(() =>
    window.location.search.includes("selftest"),
  );

  useEffect(() => {
    invoke<boolean>("is_selftest")
      .then((active) => {
        if (active) setIsSelftest(true);
      })
      .catch(() => {});

    api
      .takeUncleanExitNotice()
      .then((unclean) => {
        if (unclean) {
          useProjectStore
            .getState()
            .showStatus(
              "Fleuron closed unexpectedly during its last session. Your project database has been verified and is ready.",
              "info",
            );
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unlisten = listen("sync://workspace-changed", () => {
      const store = useProjectStore.getState();
      if (!store.project) return;
      void api
        .getLiveWorkspaceSnapshot(store.activeInterviewId)
        .then((snapshot) => useProjectStore.getState().reconcileLiveWorkspace(snapshot))
        .catch(() => {});
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  useAppInit();
  useGlobalKeyboardShortcuts();
  useMenuEvents();
  useNativeDeparture();

  return (
    <>
      {/* Tints the desktop showing through the translucent window. */}
      <div className="ambient" aria-hidden="true" />
      {project ? <WorkspaceLayout /> : <WelcomeScreen />}
      <SetupWizard />
      <UserGuidePanel />
      <AboutModal />
      <SyncSheet />
      <CoderIdentityModal />
      <JoinStudyModal />
      <SettingsSheet />
      <TrustCenterPanel />
      <ConfirmDialog />
      <UpdatePreparationOverlay />
      {/* The single typed departure dialog for every guarded action. */}
      <NoteDepartureDialog />
      {/* The single Notifications region: every toast in the app funnels
          through this one host, so one failure produces one announcement. */}
      <ToastStack />
      {/* App-local recovery: available with no study open (welcome screen). */}
      <NoteRecoveryPanel />
      {loading && <OpeningOverlay path={openingPath} />}
      {isSelftest && <SelftestRunner />}
    </>
  );
}

export default App;
