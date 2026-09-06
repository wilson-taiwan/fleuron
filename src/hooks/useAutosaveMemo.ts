import { useEffect, useRef, useState } from "react";
import { useNoteDraftStore } from "../store/note-draft-store";

/** A typing pause, not a keystroke — long enough not to save mid-word. */
export const AUTOSAVE_PAUSE_MS = 900;

export type AutosaveStatus = "idle" | "saving" | "saved" | "error";

/**
 * Automatic saving for one interview draft entry.
 *
 * Extracted from MemoPanel so removing the editing rail does not remove the
 * interview autosave with it. Truthful by construction: `saved` means the
 * entry's current revision is committed — recovery alone never qualifies,
 * and a rejected write surfaces as a sticky inline error with Retry instead
 * of an endless retry loop or a false Saved label.
 *
 * A save in flight followed by an edit schedules the newer revision after
 * the existing save through the entry's save queue; closing the sheet
 * flushes the pending save rather than dropping it.
 */
export function useAutosaveMemo(entryKey: string | null, enabled: boolean): {
  status: AutosaveStatus;
  error: string | null;
  retry: () => void;
} {
  const draftText = useNoteDraftStore((s) => (entryKey ? s.entries[entryKey]?.draftText : undefined));
  const baseText = useNoteDraftStore((s) => (entryKey ? s.entries[entryKey]?.baseText : undefined));
  const saveState = useNoteDraftStore((s) => (entryKey ? s.entries[entryKey]?.saveState : undefined));
  const saveError = useNoteDraftStore((s) => (entryKey ? s.entries[entryKey]?.saveError : undefined));
  const [status, setStatus] = useState<AutosaveStatus>("idle");
  const latest = useRef({ entryKey, enabled });
  useEffect(() => {
    latest.current = { entryKey, enabled };
  });

  const dirty = entryKey != null && draftText !== undefined && baseText !== undefined && draftText !== baseText;

  useEffect(() => {
    if (!enabled || !entryKey || !dirty) return;
    setStatus((previous) => (previous === "error" ? previous : "idle"));
    const timer = setTimeout(() => {
      setStatus("saving");
      void useNoteDraftStore
        .getState()
        .saveDraft(entryKey)
        .then((ok) => {
          const entry = useNoteDraftStore.getState().entries[entryKey];
          if (!entry) {
            setStatus("idle");
            return;
          }
          if (ok && entry.draftText === entry.baseText) {
            setStatus("saved");
          } else if (entry.saveState === "conflict") {
            setStatus("error");
          } else if (!ok) {
            setStatus("error");
          } else {
            // Newer keystrokes landed mid-save: still dirty, still scheduled.
            setStatus("idle");
          }
        })
        .catch(() => setStatus("error"));
    }, AUTOSAVE_PAUSE_MS);
    return () => clearTimeout(timer);
  }, [entryKey, enabled, draftText, dirty]);

  // Closing the sheet flushes the current pending save — it never cancels or
  // drops it. Navigation updates only the active view; it does not rebind an
  // old draft to a new interview because the key carries the interview id.
  useEffect(() => {
    const flush = () => {
      const { entryKey: key, enabled: on } = latest.current;
      if (!key || !on) return;
      const entry = useNoteDraftStore.getState().entries[key];
      if (entry && entry.draftText !== entry.baseText && entry.saveState !== "saving") {
        setStatus("saving");
        void useNoteDraftStore
          .getState()
          .saveDraft(key)
          .then((ok) => setStatus(ok ? "saved" : "error"))
          .catch(() => setStatus("error"));
      }
    };
    const onHide = () => {
      if (document.visibilityState === "hidden") flush();
    };
    window.addEventListener("blur", flush);
    document.addEventListener("visibilitychange", onHide);
    return () => {
      window.removeEventListener("blur", flush);
      document.removeEventListener("visibilitychange", onHide);
      flush();
    };
  }, []);

  useEffect(() => {
    // Error/saving mirror the entry; "saved" is earned only by a completed
    // save in this mount — a freshly resumed clean entry shows nothing
    // rather than claiming a save that never ran here.
    if (saveState === "error" || saveState === "conflict") setStatus("error");
    else if (saveState === "saving") setStatus("saving");
  }, [saveState]);

  return {
    status,
    error: status === "error" ? (saveError ?? "Could not save.") : null,
    retry: () => {
      if (!entryKey) return;
      setStatus("saving");
      void useNoteDraftStore
        .getState()
        .saveDraft(entryKey)
        .then((ok) => setStatus(ok ? "saved" : "error"))
        .catch(() => setStatus("error"));
    },
  };
}
