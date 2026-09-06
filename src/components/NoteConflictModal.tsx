import { useEffect, useRef, useState } from "react";
import { useNoteDraftStore } from "../store/note-draft-store";
import { Modal } from "./ui/Surfaces";

/**
 * Comparison for a note whose committed text changed under a dirty draft.
 *
 * Both versions survive until the user chooses: Save my version re-CASes
 * against the version shown (a further change refreshes the comparison and
 * asks again), Use saved version adopts the live text and drops the draft,
 * Keep editing / Escape leaves both untouched. While open this is the single
 * editable surface — the underlying editor disables its own textarea during
 * a conflict and restores focus afterwards.
 */
export function NoteConflictModal({
  entryKey,
  triggerLabel = "Compare versions",
}: {
  entryKey: string;
  triggerLabel?: string;
}) {
  const entry = useNoteDraftStore((s) => s.entries[entryKey]);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const draftRef = useRef<HTMLTextAreaElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) {
      draftRef.current?.focus();
      draftRef.current?.select();
    } else {
      triggerRef.current?.focus();
    }
  }, [open ]);

  if (!entry || entry.conflictText === null) return null;
  const conflictText = entry.conflictText;
  const contextLabel =
    entry.kind === "interview" ? "Interview note" : `Passage note · ${entry.context.participantLabel}`;

  const close = () => {
    setOpen(false);
    setBusy(false);
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="btn btn-secondary btn-xs"
        onClick={() => setOpen(true)}
      >
        {triggerLabel}
      </button>
      {open ? (
        <Modal open={open} onClose={close} title="This note changed elsewhere" width="max-w-2xl">
          <div className="flex min-h-0 flex-1 flex-col gap-3 px-5 py-4">
            <p className="hint text-[11.5px]">
              {contextLabel}. Your unfinished draft is on the left and stays
              editable; the current saved note is on the right and is read-only.
              Nothing is overwritten until you choose.
            </p>
            <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 md:grid-cols-2">
              <label className="flex min-h-0 flex-col gap-1">
                <span className="text-[11px] font-medium">Your unfinished draft</span>
                <textarea
                  ref={draftRef}
                  value={entry.draftText}
                  onChange={(e) => useNoteDraftStore.getState().editDraft(entry.key, e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Escape" && !e.nativeEvent.isComposing) {
                      e.stopPropagation();
                      useNoteDraftStore.getState().keepEditing(entry.key);
                      close();
                    }
                  }}
                  aria-label="Your unfinished draft"
                  className="field min-h-[160px] flex-1 text-[13px]"
                />
              </label>
              <label className="flex min-h-0 flex-col gap-1">
                <span className="text-[11px] font-medium">Current saved note</span>
                <textarea
                  value={conflictText}
                  readOnly
                  aria-label="Current saved note"
                  className="field min-h-[160px] flex-1 text-[13px] opacity-80"
                />
              </label>
            </div>
            <div className="flex flex-wrap items-center justify-end gap-1.5">
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                disabled={busy}
                onClick={() => {
                  useNoteDraftStore.getState().keepEditing(entry.key);
                  close();
                }}
              >
                Keep editing
              </button>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  void useNoteDraftStore
                    .getState()
                    .useSavedVersion(entry.key)
                    .then(() => close());
                }}
              >
                Use saved version
              </button>
              <button
                type="button"
                className="btn btn-primary btn-sm"
                disabled={busy}
                onClick={() => {
                  setBusy(true);
                  void useNoteDraftStore
                    .getState()
                    .saveMineOver(entry.key)
                    .then((ok) => {
                      // A further change refreshes the comparison and asks
                      // again; only a clean save closes.
                      const live = useNoteDraftStore.getState().entries[entry.key];
                      if (ok && live && live.saveState !== "conflict") close();
                      else setBusy(false);
                    });
                }}
              >
                {busy ? "Saving…" : "Save my version"}
              </button>
            </div>
          </div>
        </Modal>
      ) : null}
    </>
  );
}
