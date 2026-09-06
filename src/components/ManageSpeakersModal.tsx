import { useCallback, useEffect, useState } from "react";
import { Modal } from "./ui/Surfaces";
import { Icon } from "./ui/Icon";
import { api } from "../lib/api";
import { useAppStore } from "../store/app-store";
import { useProjectStore } from "../store/project-store";
import { useSpeakerRedactionOn } from "../hooks/useSpeakerDisplay";
import type { Interview, InterviewSpeakerSummary } from "../lib/types";

export interface ManageSpeakersModalProps {
  open: boolean;
  onClose: () => void;
  interview: Interview;
}

export function ManageSpeakersModal({
  open,
  onClose,
  interview,
}: ManageSpeakersModalProps) {
  const isRedactionOn = useSpeakerRedactionOn();
  const setSpeakerRedaction = useAppStore((s) => s.setSpeakerRedaction);
  const renameInterviewSpeaker = useProjectStore(
    (s) => s.renameInterviewSpeaker,
  );

  const [speakers, setSpeakers] = useState<InterviewSpeakerSummary[]>([]);
  const [loading, setLoading] = useState(false);
  const [editingSpeaker, setEditingSpeaker] = useState<string | null>(null);
  const [nameInput, setNameInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [combinePrompt, setCombinePrompt] = useState<{
    oldSpeaker: string;
    newSpeaker: string;
    turnCount: number;
    existingTurnCount: number;
  } | null>(null);

  const loadSpeakers = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await api.getInterviewSpeakers(interview.id);
      setSpeakers(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [interview.id]);

  useEffect(() => {
    if (open && !isRedactionOn) {
      void loadSpeakers();
      setEditingSpeaker(null);
      setNameInput("");
      setCombinePrompt(null);
      setError(null);
    }
  }, [open, isRedactionOn, loadSpeakers]);

  const handleStartEdit = (summary: InterviewSpeakerSummary) => {
    setEditingSpeaker(summary.speaker);
    setNameInput(summary.speaker);
    setError(null);
    setCombinePrompt(null);
  };

  const handleCancelEdit = () => {
    setEditingSpeaker(null);
    setNameInput("");
    setError(null);
    setCombinePrompt(null);
  };

  const validateName = (name: string, oldSpeaker: string): string | null => {
    const trimmed = name.trim();
    if (!trimmed) {
      return "Speaker name cannot be empty.";
    }
    if (trimmed.length > 120) {
      return "Speaker name cannot exceed 120 characters.";
    }
    if (/[\u0000-\u001F\u007F]/.test(trimmed)) {
      return "Speaker name cannot contain control characters.";
    }
    if (trimmed === oldSpeaker) {
      return null; // no-op
    }
    return null;
  };

  const executeRename = async (
    oldSpeaker: string,
    newSpeaker: string,
    expectedCount: number,
  ) => {
    setBusy(true);
    setError(null);
    try {
      await renameInterviewSpeaker(
        interview.id,
        oldSpeaker,
        newSpeaker,
        expectedCount,
      );
      setEditingSpeaker(null);
      setNameInput("");
      setCombinePrompt(null);
      await loadSpeakers();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSubmitEdit = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    if (!editingSpeaker) return;
    const trimmed = nameInput.trim();
    if (trimmed === editingSpeaker) {
      handleCancelEdit();
      return;
    }
    const valErr = validateName(trimmed, editingSpeaker);
    if (valErr) {
      setError(valErr);
      return;
    }

    const currentSummary = speakers.find((s) => s.speaker === editingSpeaker);
    const affectedCount = currentSummary ? currentSummary.turn_count : 0;

    const existingTarget = speakers.find(
      (s) => s.speaker.toLowerCase() === trimmed.toLowerCase(),
    );
    if (existingTarget && existingTarget.speaker !== editingSpeaker) {
      setCombinePrompt({
        oldSpeaker: editingSpeaker,
        newSpeaker: existingTarget.speaker,
        turnCount: affectedCount,
        existingTurnCount: existingTarget.turn_count,
      });
      return;
    }

    await executeRename(editingSpeaker, trimmed, affectedCount);
  };

  return (
    <Modal
      open={open}
      onClose={() => {
        if (!busy) onClose();
      }}
      title={`Manage Speakers — ${interview.participant_label}`}
      subtitle="Changes apply to this interview on this computer."
      width="max-w-lg"
    >
      {isRedactionOn ? (
        <div className="flex flex-col items-center gap-4 py-6 text-center">
          <div
            className="flex h-12 w-12 items-center justify-center rounded-full"
            style={{ background: "var(--warning-soft)", color: "var(--warning)" }}
          >
            <Icon name="eye" size={24} />
          </div>
          <div>
            <h3 className="text-[15px] font-semibold text-[var(--ink)]">
              Speaker name hiding is active
            </h3>
            <p
              className="mt-1 text-[13px] leading-relaxed max-w-sm"
              style={{ color: "var(--ink-2)" }}
            >
              Turn off speaker name hiding in this interview before viewing or
              editing actual speaker labels.
            </p>
          </div>
          <button
            type="button"
            className="btn btn-primary mt-2"
            onClick={() => setSpeakerRedaction(interview.id, false)}
          >
            Turn off name hiding
          </button>
        </div>
      ) : combinePrompt ? (
        <div className="flex flex-col gap-4 py-2">
          <div
            className="rounded-[10px] p-3 text-[13px] leading-relaxed"
            style={{
              background: "var(--warning-soft)",
              color: "var(--warning)",
            }}
          >
            <div className="font-semibold text-[13.5px]">Combine speakers?</div>
            <p className="mt-1">
              Renaming <strong>“{combinePrompt.oldSpeaker}”</strong> to{" "}
              <strong>“{combinePrompt.newSpeaker}”</strong> will combine{" "}
              {combinePrompt.turnCount} turn
              {combinePrompt.turnCount === 1 ? "" : "s"} into “
              {combinePrompt.newSpeaker}” (which already has{" "}
              {combinePrompt.existingTurnCount} turn
              {combinePrompt.existingTurnCount === 1 ? "" : "s"}).
            </p>
            <p className="mt-1">Both will share the same speaker label.</p>
          </div>

          {error && (
            <div
              role="alert"
              className="rounded-[8px] p-2.5 text-[12.5px]"
              style={{
                background: "var(--danger-soft)",
                color: "var(--danger)",
              }}
            >
              {error}
            </div>
          )}

          <div className="flex items-center justify-end gap-2 pt-2">
            <button
              type="button"
              className="btn"
              disabled={busy}
              onClick={() => setCombinePrompt(null)}
            >
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() =>
                void executeRename(
                  combinePrompt.oldSpeaker,
                  combinePrompt.newSpeaker,
                  combinePrompt.turnCount,
                )
              }
            >
              {busy ? "Combining…" : "Combine speakers"}
            </button>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          {error && (
            <div
              role="alert"
              className="rounded-[8px] p-2.5 text-[12.5px]"
              style={{
                background: "var(--danger-soft)",
                color: "var(--danger)",
              }}
            >
              {error}
            </div>
          )}

          {loading ? (
            <div
              className="py-8 text-center text-[13px]"
              style={{ color: "var(--ink-3)" }}
            >
              Loading speakers…
            </div>
          ) : speakers.length === 0 ? (
            <div
              className="py-8 text-center text-[13px]"
              style={{ color: "var(--ink-3)" }}
            >
              No speakers found in this interview transcript.
            </div>
          ) : (
            <div className="flex flex-col divide-y divide-[var(--border)] max-h-[380px] overflow-y-auto">
              {speakers.map((s) => {
                const isEditing = editingSpeaker === s.speaker;
                return (
                  <div key={s.speaker} className="py-2.5 flex flex-col gap-2">
                    {isEditing ? (
                      <form
                        onSubmit={(e) => void handleSubmitEdit(e)}
                        className="flex flex-col gap-2"
                      >
                        <div className="text-[12px] font-medium text-[var(--ink-2)]">
                          Renaming <strong>“{s.speaker}”</strong> ({s.turn_count}{" "}
                          turn{s.turn_count === 1 ? "" : "s"}):
                        </div>
                        <div className="flex items-center gap-2">
                          <input
                            type="text"
                            value={nameInput}
                            onChange={(e) => setNameInput(e.target.value)}
                            disabled={busy}
                            maxLength={120}
                            autoFocus
                            className="field flex-1 text-[13px]"
                            placeholder="Enter new speaker label…"
                          />
                          <button
                            type="button"
                            className="btn btn-sm"
                            disabled={busy}
                            onClick={handleCancelEdit}
                          >
                            Cancel
                          </button>
                          <button
                            type="submit"
                            className="btn btn-sm btn-primary"
                            disabled={busy || !nameInput.trim()}
                          >
                            {busy
                              ? "Renaming…"
                              : `Rename ${s.turn_count} turn${s.turn_count === 1 ? "" : "s"}`}
                          </button>
                        </div>
                      </form>
                    ) : (
                      <div className="flex items-center justify-between">
                        <div className="flex items-center gap-2.5">
                          <span className="font-semibold text-[13.5px] text-[var(--ink)]">
                            {s.speaker}
                          </span>
                          <span
                            className="rounded-full px-2 py-0.5 text-[11px] font-medium"
                            style={{
                              background: "var(--fill)",
                              color: "var(--ink-2)",
                            }}
                          >
                            {s.turn_count} turn{s.turn_count === 1 ? "" : "s"}
                          </span>
                        </div>
                        <button
                          type="button"
                          className="btn btn-sm"
                          disabled={busy || editingSpeaker !== null}
                          onClick={() => handleStartEdit(s)}
                        >
                          Rename…
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}

          <div className="flex items-center justify-end pt-2 border-t border-[var(--border)]">
            <button
              type="button"
              className="btn"
              disabled={busy}
              onClick={onClose}
            >
              Close
            </button>
          </div>
        </div>
      )}
    </Modal>
  );
}
