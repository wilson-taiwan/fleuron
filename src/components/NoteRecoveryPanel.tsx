import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { api } from "../lib/api";
import { formatRelativeTime } from "../lib/format";
import { useProjectStore } from "../store/project-store";
import { useNoteDraftStore } from "../store/note-draft-store";
import { appConfirm } from "../store/confirm-store";
import { SideSheet } from "./ui/Surfaces";
import { Icon } from "./ui/Icon";
import type { NoteDraftRecord } from "../lib/types";

/**
 * Unfinished notes: app-local crash recovery, listed newest first.
 *
 * Recovery drafts are not committed notes — they never sync and never enter
 * exports or study backups. Text and captured context render only when a
 * record is expanded; toasts and logs never carry note text. Live targets
 * offer Resume / Copy / Discard; other-project, unavailable and deleted
 * targets offer Copy / Discard with guidance and never recreate coding or
 * append to interview notes on their own.
 */

// One non-blocking notice per session when records are waiting.
let recoveredNoticeShown = false;

export function useUnfinishedNotesCount(): number {
  const [count, setCount] = useState(0);
  useEffect(() => {
    let cancelled = false;
    void api
      .listNoteDrafts()
      .then((drafts) => {
        if (!cancelled) {
          setCount(drafts.filter((d) => !d.discarded && d.draft_text !== d.base_text).length);
        }
      })
      .catch(() => {
        if (!cancelled) setCount(0);
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return count;
}

function recordLabel(record: NoteDraftRecord): string {
  if (record.kind === "interview") return "Interview note";
  if (record.segment_index !== null && record.segment_index !== undefined) {
    return `Passage ${record.segment_index + 1}`;
  }
  return "Passage note";
}

export function NoteRecoveryPanel() {
  const open = useProjectStore((s) => s.showRecoveryPanel);
  const setOpen = useProjectStore((s) => s.setShowRecoveryPanel);
  const { project, selectInterview, setSelectedSegmentId, openNoteForCoding, setShowInterviewMemo, clearAllFilters, showStatus } =
    useProjectStore(
      useShallow((s) => ({
        project: s.project,
        selectInterview: s.selectInterview,
        setSelectedSegmentId: s.setSelectedSegmentId,
        openNoteForCoding: s.openNoteForCoding,
        setShowInterviewMemo: s.setShowInterviewMemo,
        clearAllFilters: s.clearAllFilters,
        showStatus: s.showStatus,
      })),
    );
  const draftWorkspace = useNoteDraftStore((s) => s.workspace);
  const [records, setRecords] = useState<NoteDraftRecord[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [resolution, setResolution] = useState<
    Record<string, { state: "live" | "missing"; reason?: string; interviewId?: string } | undefined>
  >({});
  const [busyId, setBusyId] = useState<string | null>(null);
  const [copyError, setCopyError] = useState<string | null>(null);
  const announced = useRef(false);

  const reload = useCallback(async () => {
    setLoadError(null);
    try {
      const listed = await api.listNoteDrafts();
      // Lazily remove redundant rows (draft already equals the committed
      // text) — conditionally and silently; they hold nothing unfinished.
      const live = listed.filter((d) => !d.discarded);
      const redundant = live.filter((d) => d.draft_text === d.base_text);
      if (redundant.length > 0) {
        for (const row of redundant) {
          try {
            await api.discardNoteDraft({ draft_id: row.draft_id });
          } catch {
            // A leftover redundant row is a wart, not data loss.
          }
        }
        const remaining = live.filter((d) => d.draft_text !== d.base_text);
        setRecords(remaining);
      } else {
        setRecords(live);
      }
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : String(error));
      setRecords([]);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload, open, draftWorkspace?.epoch]);

  // One non-blocking notice per session when unfinished work is waiting —
  // never opened or committed automatically, and carrying no note text.
  useEffect(() => {
    if (recoveredNoticeShown || records === null || records.length === 0) return;
    const unfinished = records.filter((d) => d.draft_text !== d.base_text);
    if (unfinished.length === 0) return;
    recoveredNoticeShown = true;
    announced.current = true;
    showStatus("Unfinished notes recovered", "info", {
      label: "Review",
      onClick: () => setOpen(true),
    });
  }, [records, showStatus, setOpen]);

  const groups = useMemo(() => {
    const map = new Map<string, { title: string; mine: boolean; rows: NoteDraftRecord[] }>();
    for (const record of records ?? []) {
      const mine = draftWorkspace != null && record.project_key === draftWorkspace.projectKey;
      const title = record.project_title ?? (mine ? (project?.title ?? "This study") : "Another study");
      const key = `${record.project_key}`;
      const group = map.get(key) ?? { title, mine, rows: [] };
      group.rows.push(record);
      map.set(key, group);
    }
    return [...map.values()].sort((a, b) => Number(b.mine) - Number(a.mine));
  }, [records, draftWorkspace, project?.title]);

  async function resolve(record: NoteDraftRecord) {
    if (resolution[record.draft_id]) return resolution[record.draft_id]!;
    try {
      const result = await api.resolveNoteDraftTarget(record.draft_id);
      const next =
        result.status === "live-coding"
          ? { state: "live" as const, interviewId: result.interview_id }
          : result.status === "live-interview"
            ? { state: "live" as const, interviewId: record.interview_id }
            : { state: "missing" as const, reason: result.reason };
      setResolution((previous) => ({ ...previous, [record.draft_id]: next }));
      return next;
    } catch {
      const next = { state: "missing" as const, reason: "unavailable" };
      setResolution((previous) => ({ ...previous, [record.draft_id]: next }));
      return next;
    }
  }

  async function toggleExpand(record: NoteDraftRecord) {
    if (expandedId === record.draft_id) {
      setExpandedId(null);
      return;
    }
    setExpandedId(record.draft_id);
    setCopyError(null);
    await resolve(record);
  }

  async function copyRecord(record: NoteDraftRecord) {
    setCopyError(null);
    try {
      await navigator.clipboard.writeText(record.draft_text);
      showStatus("Draft text copied.", "info");
    } catch {
      setCopyError("Copy failed — select the text and copy it manually.");
    }
  }

  async function discardRecord(record: NoteDraftRecord) {
    const ok = await appConfirm({
      title: "Discard this unfinished note?",
      body: "The unfinished text will be removed from this computer. Committed notes are not affected.",
      confirmLabel: "Discard",
      cancelLabel: "Keep editing",
      destructive: true,
      dedupeKey: `discard-draft-${record.draft_id}`,
    });
    if (!ok) return;
    setBusyId(record.draft_id);
    try {
      await api.discardNoteDraft({ draft_id: record.draft_id });
      const workspace = useNoteDraftStore.getState().workspace;
      if (workspace) {
        const key = `${workspace.projectKey}::${record.kind}::${record.target_id}`;
        useNoteDraftStore.getState().removeEntry(key);
      }
      setResolution((previous) => {
        const next = { ...previous };
        delete next[record.draft_id];
        return next;
      });
      await reload();
    } catch (error) {
      showStatus(`Could not discard the draft: ${String(error)} Nothing was deleted.`, "error");
    } finally {
      setBusyId(null);
    }
  }

  async function resumeRecord(record: NoteDraftRecord) {
    const verdict = await resolve(record);
    if (verdict.state !== "live") return;
    setBusyId(record.draft_id);
    try {
      if (record.kind === "interview") {
        if (draftWorkspace && record.project_key === draftWorkspace.projectKey) {
          await selectInterview(record.interview_id);
          setShowInterviewMemo(true);
        }
      } else {
        // Reveal the target explicitly: clear active filters (retaining
        // transcript search text), select the interview and passage, then
        // open the shared inline editor on the existing draft.
        clearAllFilters();
        if (draftWorkspace && record.project_key === draftWorkspace.projectKey) {
          await selectInterview(verdict.interviewId ?? record.interview_id, record.segment_id, {
            persist: false,
          });
          if (record.segment_id) setSelectedSegmentId(record.segment_id, "jump");
          openNoteForCoding(record.target_id);
          showStatus("Filters cleared to show this note.", "info");
          setOpen(false);
        }
      }
    } finally {
      setBusyId(null);
    }
  }

  return (
    <SideSheet
      open={open}
      onClose={() => setOpen(false)}
      title="Unfinished notes"
      subtitle="Local crash recovery on this computer — never synced, never exported"
      width="max-w-lg"
    >
      <div className="flex min-h-0 flex-1 flex-col px-5 py-4">
        {loadError ? (
          <div role="alert" className="rounded-md bg-[var(--danger,#b03a34)]/10 px-2.5 py-1.5 text-[11.5px] text-[var(--danger,#b03a34)]">
            Could not load unfinished notes: {loadError}
          </div>
        ) : records === null ? (
          <p className="hint text-[12px]">Loading…</p>
        ) : records.length === 0 ? (
          <p className="hint text-[12px]">No unfinished notes</p>
        ) : (
          <ul className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
            {groups.map((group) => (
              <li key={group.title}>
                <h3 className="eyebrow mb-1.5">{group.title}</h3>
                <ul className="flex flex-col gap-1.5">
                  {group.rows.map((record) => {
                    const expanded = expandedId === record.draft_id;
                    const verdict = resolution[record.draft_id];
                    const live = verdict?.state === "live";
                    const missingReason = verdict?.state === "missing" ? verdict.reason : undefined;
                    return (
                      <li
                        key={record.draft_id}
                        className="rounded-[12px] border border-[var(--border)] bg-[var(--surface)]"
                      >
                        <button
                          type="button"
                          onClick={() => void toggleExpand(record)}
                          aria-expanded={expanded}
                          className="flex w-full items-center gap-2 px-3 py-2 text-left"
                        >
                          <Icon name="note" size={13} />
                          <span className="min-w-0 flex-1">
                            <span className="block truncate text-[13px] font-medium">
                              {recordLabel(record)} · {record.participant_label}
                            </span>
                            <span className="hint block text-[11px]">
                              Last edited {formatRelativeTime(record.updated_at)}
                            </span>
                          </span>
                          <Icon name={expanded ? "chevronDown" : "chevronRight"} size={13} />
                        </button>
                        {expanded ? (
                          <div className="border-t border-[var(--g-rim)]/60 px-3 py-2.5">
                            <p className="select-text whitespace-pre-wrap break-words text-[12.5px]">
                              {record.draft_text || <span className="hint">(empty draft)</span>}
                            </p>
                            {record.quote_text ? (
                              <p className="mt-1.5 select-text line-clamp-3 font-serif italic text-[11.5px] text-[var(--ink-3)]">
                                “{record.quote_text}”
                              </p>
                            ) : null}
                            {verdict === undefined ? (
                              <p className="hint mt-1.5 text-[11px]">Checking the original…</p>
                            ) : live ? (
                              <p className="hint mt-1.5 text-[11px]">
                                {group.mine
                                  ? "The original is available in the open study."
                                  : "Open the original study to resume."}
                              </p>
                            ) : missingReason === "deleted" ? (
                              <p className="hint mt-1.5 text-[11px]">
                                The original coding is no longer available. Your text stays here
                                until you copy or discard it.
                              </p>
                            ) : (
                              <p className="hint mt-1.5 text-[11px]">
                                Open the original study to resume.
                              </p>
                            )}
                            {copyError && expanded ? (
                              <p role="alert" className="mt-1.5 text-[11px] text-[var(--danger,#b03a34)]">
                                {copyError}
                              </p>
                            ) : null}
                            <div className="mt-2 flex flex-wrap items-center gap-1.5">
                              {live && group.mine ? (
                                <button
                                  type="button"
                                  className="btn btn-primary btn-xs"
                                  disabled={busyId === record.draft_id}
                                  onClick={() => void resumeRecord(record)}
                                >
                                  Resume
                                </button>
                              ) : null}
                              <button
                                type="button"
                                className="btn btn-secondary btn-xs"
                                disabled={busyId === record.draft_id}
                                onClick={() => void copyRecord(record)}
                              >
                                Copy text
                              </button>
                              <button
                                type="button"
                                className="btn btn-ghost btn-xs text-[var(--danger,#b03a34)]"
                                disabled={busyId === record.draft_id}
                                onClick={() => void discardRecord(record)}
                              >
                                Discard
                              </button>
                            </div>
                          </div>
                        ) : null}
                      </li>
                    );
                  })}
                </ul>
              </li>
            ))}
          </ul>
        )}
        <p className="hint mt-3 text-[11px]">
          Recovery drafts stay on this computer and are not included in study backups or exports.
        </p>
      </div>
    </SideSheet>
  );
}
