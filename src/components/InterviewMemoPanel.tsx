import { useEffect, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useProjectStore } from "../store/project-store";
import { draftKey, useNoteDraftStore } from "../store/note-draft-store";
import { SideSheet } from "./ui/Surfaces";
import { useAutosaveMemo } from "../hooks/useAutosaveMemo";
import { NoteConflictModal } from "./NoteConflictModal";

/**
 * The interview-wide analytic memo.
 *
 * Lived at the bottom of the right rail until 0.16.0, directly beneath a note
 * about one passage. Two different altitudes of thought in one column, and the
 * cheaper one to reach won: cross-cutting reflection got written into a note
 * attached to whichever passage happened to be selected. It is a sheet now, so
 * opening it is a deliberate change of gear.
 *
 * Saving is automatic after a typing pause through the shared draft entry:
 * the same revision CAS, conflict comparison and local recovery as passage
 * notes. `Saved` means the current revision is committed; recovery alone
 * never qualifies.
 */
export function InterviewMemoPanel() {
  const {
    hubMemo,
    setHubMemo,
    showInterviewMemo,
    setShowInterviewMemo,
    interviews,
    activeInterviewId,
  } = useProjectStore(
    useShallow((s) => ({
      hubMemo: s.hubMemo,
      setHubMemo: s.setHubMemo,
      showInterviewMemo: s.showInterviewMemo,
      setShowInterviewMemo: s.setShowInterviewMemo,
      interviews: s.interviews,
      activeInterviewId: s.activeInterviewId,
    })),
  );
  const workspace = useNoteDraftStore((s) => s.workspace);
  const frozen = useNoteDraftStore((s) => s.frozen);
  const [beginError, setBeginError] = useState<string | null>(null);

  const interview = interviews.find((i) => i.id === activeInterviewId);
  const entryKey =
    workspace && activeInterviewId
      ? draftKey(workspace.projectKey, "interview", activeInterviewId)
      : null;
  const entry = useNoteDraftStore((s) => (entryKey ? s.entries[entryKey] : undefined));

  // Opening the sheet starts (or resumes) the interview draft. Changes made
  // while another interview is active save by captured identity — the key
  // carries the interview id, so navigation cannot rebind the draft.
  useEffect(() => {
    if (!showInterviewMemo || !workspace || !activeInterviewId || !interview) return;
    setBeginError(null);
    void useNoteDraftStore
      .getState()
      .beginDraft({
        kind: "interview",
        targetId: activeInterviewId,
        interviewId: activeInterviewId,
        participantLabel: interview.participant_label,
      })
      .catch((error: unknown) => {
        setBeginError(error instanceof Error ? error.message : String(error));
      });
  }, [showInterviewMemo, workspace?.projectKey, workspace?.epoch, activeInterviewId]);

  const autosave = useAutosaveMemo(entryKey, showInterviewMemo);
  const conflicted = entry?.saveState === "conflict" && entry.conflictText !== null;
  const missing =
    entry?.targetMissing ||
    entry?.saveState === "error" ||
    (entryKey != null && beginError !== null && /no longer available/i.test(beginError));
  const recovering = entry?.recoveryState === "backing-up";
  const recoveryFailed =
    entry?.recoveryState === "error" || entry?.recoveryState === "unavailable";

  return (
    <>
      <SideSheet
        open={showInterviewMemo}
        onClose={() => setShowInterviewMemo(false)}
        title="Notes on this interview"
        subtitle={
          interview
            ? `${interview.participant_label} — yours alone, never synced`
            : undefined
        }
        width="max-w-lg"
        actions={
          <span className="hint text-[11px]" role="status">
            {autosave.status === "saving"
              ? "Saving…"
              : autosave.status === "saved"
                ? "Saved"
                : autosave.status === "error"
                  ? "Not saved"
                  : null}
          </span>
        }
      >
        <div className="flex min-h-0 flex-1 flex-col px-5 py-4">
          <p className="hint text-[11.5px]">
            About the interview as a whole, not any one passage. Memo fields are excluded from collaboration sync. Exports may include them.
          </p>
          {beginError && !missing ? (
            <div role="alert" className="mt-2 rounded-md bg-[var(--danger,#b03a34)]/10 px-2.5 py-1.5 text-[11.5px] text-[var(--danger,#b03a34)]">
              {beginError}
            </div>
          ) : null}
          {missing ? (
            <div role="alert" className="mt-2 rounded-md bg-[var(--danger,#b03a34)]/10 px-2.5 py-1.5 text-[11.5px] text-[var(--danger,#b03a34)]">
              This interview is no longer available. Your unfinished text is kept in
              Unfinished notes — copy it before discarding.
            </div>
          ) : null}
          {conflicted ? (
            <div className="mt-2 flex items-center gap-2 rounded-md bg-[var(--fill)] px-2.5 py-1.5 text-[11.5px]">
              <span className="flex-1">This note changed elsewhere. Compare before saving.</span>
              <NoteConflictModal entryKey={entryKey!} triggerLabel="Compare" />
            </div>
          ) : null}
          <textarea
            value={entry?.draftText ?? hubMemo}
            onChange={(e) => {
              if (!frozen) setHubMemo(e.target.value);
            }}
            placeholder="Cross-cutting reflections on this interview…"
            aria-label="Interview analytic memo"
            className="field mt-2 min-h-0 flex-1 text-[13px]"
            // Disabled until the draft entry exists: typing into the void
            // before begin resolves would be clobbered by the resume.
            disabled={entry == null || conflicted || frozen}
          />
          <div className="mt-2 flex min-h-[18px] items-center gap-2 text-[11px] text-[var(--ink-3)]">
            {recovering ? (
              <span role="status">Backing up draft…</span>
            ) : recoveryFailed && entry ? (
              <>
                <span role="alert">Draft not backed up — your text is held in memory only.</span>
                <button
                  type="button"
                  className="btn btn-ghost btn-xs"
                  onClick={() => useNoteDraftStore.getState().retryRecovery(entry.key)}
                >
                  Retry
                </button>
              </>
            ) : autosave.status === "error" && autosave.error ? (
              <>
                <span role="alert">{autosave.error}</span>
                <button type="button" className="btn btn-ghost btn-xs" onClick={autosave.retry}>
                  Retry
                </button>
              </>
            ) : null}
          </div>
        </div>
      </SideSheet>
    </>
  );
}
