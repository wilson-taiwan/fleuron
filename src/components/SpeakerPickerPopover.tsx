import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "./ui/Icon";
import type { TranscriptSegment } from "../lib/types";

export interface SpeakerPickerPopoverProps {
  segment: TranscriptSegment;
  allSegments: TranscriptSegment[];
  anchorRect: DOMRect;
  showSpeaker: (speaker: string) => string;
  onApply: (
    segmentId: string,
    newSpeaker: string,
    includeFollowing: boolean,
  ) => Promise<void>;
  onClose: () => void;
}

export function SpeakerPickerPopover({
  segment,
  allSegments,
  anchorRect,
  showSpeaker,
  onApply,
  onClose,
}: SpeakerPickerPopoverProps) {
  const popoverRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const [stage, setStage] = useState<"pick" | "scope">("pick");
  const [selectedSpeaker, setSelectedSpeaker] = useState<string>("");
  const [newSpeakerName, setNewSpeakerName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const distinctSpeakers = useMemo(() => {
    const list: string[] = [];
    for (const s of allSegments) {
      if (s.speaker && !list.includes(s.speaker)) {
        list.push(s.speaker);
      }
    }
    return list;
  }, [allSegments]);

  const targetIdx = useMemo(
    () => allSegments.findIndex((s) => s.id === segment.id),
    [allSegments, segment.id],
  );

  const hasLaterMatching = useMemo(() => {
    if (targetIdx < 0) return false;
    return allSegments.slice(targetIdx + 1).some((s) => s.speaker === segment.speaker);
  }, [allSegments, targetIdx, segment.speaker]);

  // Position calculation with vertical clamping
  const pos = useMemo(() => {
    const gap = 6;
    const popoverWidth = 260;
    const estimatedHeight = 280;
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    let left = anchorRect.left;
    if (left + popoverWidth > viewportWidth - 16) {
      left = Math.max(16, viewportWidth - popoverWidth - 16);
    }
    let top = anchorRect.bottom + gap;
    if (top + estimatedHeight > viewportHeight - 16 && anchorRect.top - estimatedHeight - gap > 16) {
      top = Math.max(16, anchorRect.top - estimatedHeight - gap);
    }
    return { top, left };
  }, [anchorRect]);

  // Outside click and Escape
  useEffect(() => {
    const handleDown = (e: MouseEvent) => {
      if (busy) return;
      if (popoverRef.current && !popoverRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKey = (e: KeyboardEvent) => {
      if (busy) return;
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("mousedown", handleDown, true);
    window.addEventListener("keydown", handleKey);
    return () => {
      window.removeEventListener("mousedown", handleDown, true);
      window.removeEventListener("keydown", handleKey);
    };
  }, [onClose, busy]);

  const handleSelectSpeaker = async (rawSpeaker: string) => {
    const trimmed = rawSpeaker.trim();
    if (!trimmed || trimmed === segment.speaker) {
      onClose();
      return;
    }
    if (hasLaterMatching) {
      setSelectedSpeaker(trimmed);
      setStage("scope");
    } else {
      setBusy(true);
      setError(null);
      try {
        await onApply(segment.id, trimmed, false);
        onClose();
      } catch (err) {
        setError(String(err));
        setBusy(false);
      }
    }
  };

  const handleApplyScope = async (includeFollowing: boolean) => {
    setBusy(true);
    setError(null);
    try {
      await onApply(segment.id, selectedSpeaker, includeFollowing);
      onClose();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  return createPortal(
    <div
      ref={popoverRef}
      role="dialog"
      aria-label="Change speaker label"
      className="glass-pop anim-rise fixed z-[90] w-[260px] flex flex-col p-2.5 shadow-xl select-none"
      style={{
        top: pos.top,
        left: pos.left,
      }}
    >
      {stage === "pick" ? (
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between pb-1 border-b border-[var(--g-rim)]/50">
            <span className="text-[11.5px] font-semibold text-[var(--ink)]">
              Change speaker
            </span>
            <span className="text-[10px] text-[var(--ink-4)]">
              This computer only
            </span>
          </div>

          <div className="max-h-[180px] overflow-y-auto flex flex-col gap-0.5 py-0.5">
            {distinctSpeakers.map((spk) => {
              const isCurrent = spk === segment.speaker;
              return (
                <button
                  key={spk}
                  type="button"
                  onClick={() => void handleSelectSpeaker(spk)}
                  disabled={busy}
                  className={`flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-[12px] transition-colors ${
                    isCurrent
                      ? "bg-[var(--fill-on)] text-[var(--accent)] font-medium"
                      : "hover:bg-[var(--fill)] text-[var(--ink)]"
                  }`}
                >
                  <span className="truncate">{showSpeaker(spk)}</span>
                  {isCurrent && (
                    <span className="text-[10.5px] opacity-75 font-normal">
                      (current)
                    </span>
                  )}
                </button>
              );
            })}
          </div>

          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (newSpeakerName.trim()) {
                void handleSelectSpeaker(newSpeakerName.trim());
              }
            }}
            className="flex items-center gap-1.5 pt-1 border-t border-[var(--g-rim)]/50"
          >
            <input
              ref={inputRef}
              type="text"
              value={newSpeakerName}
              onChange={(e) => setNewSpeakerName(e.target.value)}
              placeholder="New speaker…"
              disabled={busy}
              className="field field-sm flex-1 text-[11.5px]"
            />
            <button
              type="submit"
              disabled={busy || !newSpeakerName.trim()}
              className="btn btn-primary btn-sm px-2 text-[11.5px]"
            >
              Add
            </button>
          </form>
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-1.5 pb-1 border-b border-[var(--g-rim)]/50">
            <button
              type="button"
              onClick={() => {
                setStage("pick");
                setError(null);
              }}
              disabled={busy}
              className="grid h-5 w-5 place-items-center rounded text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--fill)]"
              title="Back"
            >
              <Icon name="arrowLeft" size={12} />
            </button>
            <span className="text-[11.5px] font-semibold text-[var(--ink)] truncate">
              Change to “{showSpeaker(selectedSpeaker)}”
            </span>
          </div>

          <div className="flex flex-col gap-1.5 py-1">
            <button
              type="button"
              onClick={() => void handleApplyScope(false)}
              disabled={busy}
              className="btn btn-ghost w-full justify-start text-[12px] py-1.5 text-left"
            >
              This turn only
            </button>
            <button
              type="button"
              onClick={() => void handleApplyScope(true)}
              disabled={busy}
              className="btn btn-ghost w-full justify-start text-[12px] py-1.5 text-left font-medium text-[var(--accent)]"
            >
              This turn and following {showSpeaker(segment.speaker)} turns
            </button>
          </div>

          <div className="pt-1 border-t border-[var(--g-rim)]/50 text-[10px] text-[var(--ink-4)] text-center">
            Applies immediately on this computer
          </div>
        </div>
      )}

      {error && (
        <div role="alert" className="mt-1.5 text-[11px] text-red-500 font-medium">
          {error}
        </div>
      )}
    </div>,
    document.body,
  );
}
