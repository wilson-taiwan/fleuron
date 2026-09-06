import { useEffect, useRef } from "react";
import { useNoteDepartureStore } from "../store/note-departure-store";
import { NoteConflictModal } from "./NoteConflictModal";
import { Modal } from "./ui/Surfaces";

/**
 * The one app-level departure dialog. Every guarded action — study close,
 * study replacement, restore/reset, export, quit, update install — funnels
 * through the typed choice controller behind it.
 *
 * Escape, scrim-click and window close of this prompt mean Cancel: nothing
 * is approved by dismissing the question. Double-clicking cannot duplicate
 * the operation — buttons disable while work runs.
 */
export function NoteDepartureDialog() {
  const prompt = useNoteDepartureStore((s) => s.prompt);
  const choose = useNoteDepartureStore((s) => s.choose);
  const previouslyFocused = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (prompt) {
      previouslyFocused.current =
        document.activeElement instanceof HTMLElement ? document.activeElement : null;
    } else if (previouslyFocused.current) {
      previouslyFocused.current.focus();
      previouslyFocused.current = null;
    }
  }, [prompt !== null]);

  if (!prompt) return null;
  const isExport = prompt.kind === "export";
  const working = prompt.phase === "working";

  return (
    <Modal
      open
      onClose={() => {
        if (!working) choose("cancelled");
      }}
      title={prompt.labels.title}
      subtitle={prompt.labels.body}
    >
      <div className="flex flex-col gap-3 px-7 py-5">
        {prompt.error ? (
          <div
            role="alert"
            className="rounded-md bg-[var(--danger,#b03a34)]/10 px-2.5 py-1.5 text-[12px] text-[var(--danger,#b03a34)]"
          >
            {prompt.error}
          </div>
        ) : null}
        {prompt.dirtyLabels.length > 0 ? (
          <ul className="flex max-h-40 flex-col gap-1 overflow-y-auto rounded-[12px] border border-[var(--border)] bg-[var(--surface)] p-2.5">
            {prompt.dirtyLabels.map((label) => (
              <li key={label} className="text-[12.5px]">
                {label}
              </li>
            ))}
          </ul>
        ) : null}
        {prompt.conflictKeys.length > 0 ? (
          <div className="flex flex-col gap-1.5">
            <p className="text-[12.5px] font-medium">
              {prompt.conflictKeys.length === 1
                ? "This note changed elsewhere — compare before continuing."
                : "These notes changed elsewhere — compare before continuing."}
            </p>
            {prompt.conflictKeys.map((key) => (
              <div key={key} className="flex items-center gap-2">
                <NoteConflictModal entryKey={key} triggerLabel="Compare" />
              </div>
            ))}
          </div>
        ) : null}
        {prompt.orphanLabels.length > 0 ? (
          <p className="hint text-[12px]">
            {prompt.orphanLabels.length === 1
              ? "1 recovered draft has no coding and will remain in Unfinished notes."
              : `${prompt.orphanLabels.length} recovered drafts have no coding and will remain in Unfinished notes.`}{" "}
            {isExport ? "They are excluded from this export." : "They are kept, not deleted."}
          </p>
        ) : null}
        {prompt.missingKept > 0 ? (
          <p className="hint text-[12px]">
            {prompt.missingKept === 1
              ? "1 note's original is gone; its text stays in Unfinished notes."
              : `${prompt.missingKept} notes' originals are gone; their text stays in Unfinished notes.`}
          </p>
        ) : null}
        <div className="flex flex-wrap items-center justify-end gap-1.5">
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            disabled={working}
            onClick={() => choose("cancelled")}
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            disabled={working}
            onClick={() => choose(isExport ? "saved-only" : "discard")}
          >
            {prompt.labels.discardLabel}
          </button>
          <button
            type="button"
            className="btn btn-primary btn-sm"
            disabled={working}
            onClick={() => choose(isExport ? "save-export" : "save")}
          >
            {working ? "Saving…" : prompt.labels.saveLabel}
          </button>
        </div>
      </div>
    </Modal>
  );
}
