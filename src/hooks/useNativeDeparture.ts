import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type { NativeDepartureRequest } from "../lib/types";
import { useProjectStore } from "../store/project-store";
import { useNoteDepartureStore } from "../store/note-departure-store";

/**
 * Native close/quit guard (menu quit, Cmd+Q, Alt+F4, window close).
 *
 * The backend holds the native event and emits one intent; the frontend runs
 * the same typed Save/Discard/Cancel preflight as every other departure and
 * answers exactly once. An approved answer replays the original native action
 * through `complete_note_departure` (one-use, so the replay does not prompt
 * recursively); a cancelled answer leaves markers and the connection intact.
 * There is no timeout auto-answer: forced OS termination stays covered by
 * acknowledged recovery writes only.
 *
 * The existing unsent-sync confirmation stays after draft approval: when it
 * is needed, the intent is held while the CloseProjectModal answers, and its
 * answer completes the intent.
 */
export function useNativeDeparture() {
  useEffect(() => {
    let disposed = false;
    void listen<NativeDepartureRequest>("notes://departure-requested", (event) => {
      if (disposed) return;
      const { intent_id, kind } = event.payload;
      void (async () => {
        const store = useProjectStore.getState();
        if (!store.project) {
          await api.completeNoteDeparture(intent_id, true).catch(() => {});
          return;
        }
        const gate = await useNoteDepartureStore
          .getState()
          .requestDeparture(kind === "quit" ? "quit" : "close-study");
        if (!gate.proceed) {
          await api.completeNoteDeparture(intent_id, false).catch(() => {});
          return;
        }
        // Consumed here: the replay below (or the held sync confirmation)
        // carries its own authority, and no leftover may approve a later,
        // different close.
        const draftApproved = useNoteDepartureStore.getState().consumeCloseApproval();
        // Unsent-sync confirmation after draft approval, as in the manual
        // close path. Cancelling it keeps the study open (and answers the
        // held intent as cancelled from the modal's dismiss path).
        try {
          const status = await api.syncStatus();
          if (status.signedIn && status.pendingChanges > 0) {
            useProjectStore.setState({
              showCloseConfirm: true,
              closeDraftApproved: draftApproved,
              pendingNativeDeparture: { intentId: intent_id, kind },
            });
            return;
          }
        } catch {
          // Sync not set up: nothing outstanding to warn about.
        }
        if (kind === "close") {
          await useProjectStore.getState().closeProject({ draftApproved });
        }
        await api.completeNoteDeparture(intent_id, true).catch(() => {});
      })();
    }).then((unlisten) => {
      if (disposed) unlisten();
    });
    return () => {
      disposed = true;
    };
  }, []);
}
