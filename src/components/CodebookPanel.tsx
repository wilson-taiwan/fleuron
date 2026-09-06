import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useShallow } from "zustand/react/shallow";
import { useProjectStore } from "../store/project-store";
import { useAppStore } from "../store/app-store";
import { nextCodeColor, textOnSolid } from "../lib/code-colors";
import { Icon } from "./ui/Icon";
import { Tooltip } from "./ui/Tooltip";
import { CodeEditorModal } from "./CodeEditorModal";
import { NoteHoverCard, type NoteHoverCardTarget } from "./NoteHoverCard";
import { openContextMenu } from "./ui/ContextMenu";
import type { MenuItemSpec } from "./ui/Menu";
import { canNest } from "../lib/code-drag";
import { codeMatchesQuery } from "../lib/code-search";
import {
  computeDragState,
  resolveDropTarget,
  shouldStartDrag,
  type DragPointerState,
} from "../lib/pointer-drag";
import type { Code } from "../lib/types";

export function CodebookPanel() {
  const {
    codes,
    selectedCodeIds,
    toggleCodeOnTarget,
    selectedSegmentId,
    setSelectedSegmentId,
    codedSegments,
    segments,
    activeCoder,
    recentCodeIds,
    addCode,
    updateCode,
    reparentCode,
    deleteCode,
    restoreCode,
    loadRetiredCodes,
  } = useProjectStore(
    useShallow((s) => ({
      codes: s.codes,
      selectedCodeIds: s.selectedCodeIds,
      toggleCodeOnTarget: s.toggleCodeOnTarget,
      selectedSegmentId: s.selectedSegmentId,
      setSelectedSegmentId: s.setSelectedSegmentId,
      codedSegments: s.codedSegments,
      segments: s.segments,
      activeCoder: s.activeCoder,
      recentCodeIds: s.recentCodeIds,
      addCode: s.addCode,
      updateCode: s.updateCode,
      reparentCode: s.reparentCode,
      deleteCode: s.deleteCode,
      restoreCode: s.restoreCode,
      loadRetiredCodes: s.loadRetiredCodes,
    })),
  );
  const intent = useAppStore((s) => s.intent);
  const setIntent = useAppStore((s) => s.setIntent);

  const [expanded, setExpanded] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [sortBy, setSortBy] = useState<"name" | "usage" | "recent">("name");
  const [hoverTarget, setHoverTarget] = useState<NoteHoverCardTarget | null>(null);
  const [activeDrag, setActiveDrag] = useState<DragPointerState | null>(null);
  const dragSessionRef = useRef<{
    code: Code;
    startX: number;
    startY: number;
    pointerId: number;
    dragging: boolean;
  } | null>(null);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const hoverTimerRef = useRef<number | null>(null);

  const clearHoverTimer = () => {
    if (hoverTimerRef.current) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  };

  const handlePointerDown = (
    e: React.PointerEvent<HTMLButtonElement>,
    code: Code,
  ) => {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    dragSessionRef.current = {
      code,
      startX: e.clientX,
      startY: e.clientY,
      pointerId: e.pointerId,
      dragging: false,
    };
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLButtonElement>) => {
    const session = dragSessionRef.current;
    if (!session) return;

    if (!session.dragging) {
      if (shouldStartDrag(session.startX, session.startY, e.clientX, e.clientY)) {
        session.dragging = true;
      } else {
        return;
      }
    }

    const container = scrollContainerRef.current;
    if (container) {
      const rect = container.getBoundingClientRect();
      if (e.clientY < rect.top + 40) {
        container.scrollTop -= 8;
      } else if (e.clientY > rect.bottom - 40) {
        container.scrollTop += 8;
      }
    }

    const el = document.elementFromPoint(e.clientX, e.clientY);
    const target = resolveDropTarget(el);
    const dragState = computeDragState({
      draggedCode: session.code,
      pointerX: e.clientX,
      pointerY: e.clientY,
      targetCodeId: target.targetCodeId,
      isOverTopLevelZone: target.isTopLevelZone,
      allCodes: codes,
    });
    setActiveDrag(dragState);
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLButtonElement>) => {
    const session = dragSessionRef.current;
    dragSessionRef.current = null;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {}

    if (activeDrag && session?.dragging) {
      if (activeDrag.validity.valid) {
        if (activeDrag.isOverTopLevelZone) {
          void reparentCode(session.code.id, null);
        } else if (activeDrag.targetCodeId) {
          void reparentCode(session.code.id, activeDrag.targetCodeId);
        }
      }
    }
    setActiveDrag(null);
  };

  const handlePointerCancel = () => {
    dragSessionRef.current = null;
    setActiveDrag(null);
  };

  /**
   * Where each code is used, in transcript order, including the memo and codingId for hover preview.
   */
  const usageByCode = (() => {
    const byId = new Map(segments.map((seg) => [seg.id, seg]));
    const out = new Map<
      string,
      {
        key: string;
        segmentId: string;
        codingId: string;
        number: number;
        quote: string;
        coder: string;
        memo: string;
      }[]
    >();
    for (const row of codedSegments) {
      const seg = byId.get(row.segment_id);
      if (!seg) continue;
      const span =
        row.char_start != null && row.char_end != null
          ? seg.text.slice(row.char_start, row.char_end)
          : seg.text;
      for (const codeId of row.code_ids) {
        const list = out.get(codeId) ?? [];
        list.push({
          key: `${row.id}-${codeId}`,
          segmentId: seg.id,
          codingId: row.id,
          number: seg.segment_index + 1,
          quote: span.length > 90 ? span.slice(0, 90).trimEnd() + "…" : span,
          coder: row.coder_name,
          memo: row.memo ?? "",
        });
        out.set(codeId, list);
      }
    }
    for (const list of out.values()) list.sort((a, b) => a.number - b.number);
    return out;
  })();

  const rollupUsageCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const code of codes) {
      const selfCount = code.usage_count;
      const children = codes.filter((c) => c.parent_id === code.id);
      const childCount = children.reduce((sum, c) => sum + c.usage_count, 0);
      counts.set(code.id, selfCount + childCount);
    }
    return counts;
  }, [codes]);

  const filteredAndSortedCodes = useMemo(() => {
    const q = searchQuery.toLowerCase().trim();
    let list = codes.filter((c) => codeMatchesQuery(c, q));

    if (sortBy === "name") {
      if (!q) {
        const topLevel = list.filter((c) => !c.parent_id).sort((a, b) => a.name.localeCompare(b.name));
        const out: Code[] = [];
        for (const parent of topLevel) {
          out.push(parent);
          const children = list.filter((c) => c.parent_id === parent.id).sort((a, b) => a.name.localeCompare(b.name));
          out.push(...children);
        }
        const seen = new Set(out.map((c) => c.id));
        for (const code of list) {
          if (!seen.has(code.id)) out.push(code);
        }
        return out;
      }
      return [...list].sort((a, b) => a.name.localeCompare(b.name));
    }
    if (sortBy === "usage") {
      return [...list].sort((a, b) => (rollupUsageCounts.get(b.id) ?? b.usage_count) - (rollupUsageCounts.get(a.id) ?? a.usage_count));
    }
    if (sortBy === "recent") {
      return [...list].sort((a, b) => {
        const idxA = recentCodeIds.indexOf(a.id);
        const idxB = recentCodeIds.indexOf(b.id);
        const posA = idxA === -1 ? 9999 : idxA;
        const posB = idxB === -1 ? 9999 : idxB;
        return posA - posB;
      });
    }
    return list;
  }, [codes, searchQuery, sortBy, recentCodeIds, rollupUsageCounts]);

  const [newCodeName, setNewCodeName] = useState("");
  const [showAdd, setShowAdd] = useState(false);
  const [editing, setEditing] = useState<Code | null>(null);
  const [retired, setRetired] = useState<Code[]>([]);
  const [showRetired, setShowRetired] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const refreshRetired = useCallback(() => {
    loadRetiredCodes()
      .then(setRetired)
      .catch(() => setRetired([]));
  }, [loadRetiredCodes]);

  useEffect(() => {
    refreshRetired();
  }, [refreshRetired, codes]);

  useEffect(() => {
    if (intent !== "add-code") return;
    setIntent(null);
    setShowAdd(true);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [intent, setIntent]);

  function codeMenu(code: Code): MenuItemSpec[] {
    const selected = selectedCodeIds.includes(code.id);
    const validParents = codes.filter((c) => canNest(code, c, codes).valid);
    const codeHasChildren = codes.some(
      (c) => c.parent_id === code.id && !c.is_retired,
    );

    const items: MenuItemSpec[] = [
      {
        label: selected ? "Remove from this passage" : "Apply to this passage",
        icon: "check" as const,
        onSelect: () => void toggleCodeOnTarget(code.id),
        disabled: !selectedSegmentId,
      },
      {
        label: "Edit code…",
        icon: "note" as const,
        onSelect: () => setEditing(code),
      },
    ];

    if (code.parent_id !== null) {
      items.push({
        label: "Move to top level",
        icon: "arrowLeft" as const,
        onSelect: () => void reparentCode(code.id, null),
      });
    }

    if (codeHasChildren) {
      items.push({
        label: "Move into",
        icon: "arrowRight" as const,
        disabled: true,
        shortcut: "(has sub-codes)",
        onSelect: () => {},
      });
    } else if (validParents.length > 0) {
      items.push({
        label: "Move into",
        icon: "arrowRight" as const,
        children: validParents.map((parent) => ({
          label: parent.name,
          icon: "code" as const,
          onSelect: () => void reparentCode(code.id, parent.id),
        })),
        onSelect: () => {},
      });
    }

    items.push(
      {
        label: "Retire code",
        icon: "eye" as const,
        onSelect: () => deleteCode(code.id, "retire"),
      },
      {
        label: "Delete code…",
        icon: "trash" as const,
        onSelect: () => setEditing(code),
        destructive: true,
      },
    );

    return items;
  }

  async function handleAdd() {
    const name = newCodeName.trim();
    if (!name) return;
    await addCode(name, nextCodeColor(codes.length));
    setNewCodeName("");
    inputRef.current?.focus();
  }

  const handleRowKeyDown = (e: React.KeyboardEvent<HTMLElement>, code: Code) => {
    const isCmdOrCtrl = e.metaKey || e.ctrlKey;
    if (isCmdOrCtrl && e.altKey) {
      if (e.key === "ArrowRight") {
        e.preventDefault();
        const index = filteredAndSortedCodes.findIndex((c) => c.id === code.id);
        if (index > 0) {
          for (let i = index - 1; i >= 0; i--) {
            const candidate = filteredAndSortedCodes[i];
            if (canNest(code, candidate, codes).valid) {
              void reparentCode(code.id, candidate.id);
              break;
            }
          }
        }
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        if (code.parent_id !== null) {
          void reparentCode(code.id, null);
        }
      }
    }
  };

  return (
    <aside
      data-testid="codebook-panel"
      className="glass-panel flex h-full min-h-0 flex-col overflow-hidden"
    >
      <div className="flex items-center justify-between px-3.5 pb-2 pt-3">
        <h2 className="eyebrow">Codebook</h2>
        <span className="flex items-center gap-1">
          {codes.length > 0 && (
            <span
              className="chip text-[11px] tabular-nums"
              data-testid="codebook-count"
              aria-label={
                searchQuery.trim()
                  ? `${filteredAndSortedCodes.length} of ${codes.length} codes match`
                  : `${codes.length} codes`
              }
              title={
                searchQuery.trim()
                  ? `${filteredAndSortedCodes.length} of ${codes.length} codes match`
                  : undefined
              }
            >
              {searchQuery.trim()
                ? `${filteredAndSortedCodes.length} of ${codes.length}`
                : `${codes.length}`}
            </span>
          )}
        </span>
      </div>

      {codes.length > 0 && (
        <div className="px-2.5 pb-2 space-y-1.5 border-b border-[var(--g-rim)]/50">
          <div className="relative flex items-center">
            <span className="pointer-events-none absolute left-2 text-[var(--ink-4)]">
              <Icon name="search" size={12} />
            </span>
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Find code…"
              aria-label="Find code"
              className="field field-sm w-full pl-6 pr-6 text-[12px]"
            />
            {searchQuery && (
              <button
                type="button"
                onClick={() => setSearchQuery("")}
                aria-label="Clear code search"
                className="absolute right-1.5 grid h-4 w-4 place-items-center rounded text-[var(--ink-4)] hover:text-[var(--ink)]"
              >
                <Icon name="close" size={10} />
              </button>
            )}
          </div>

          <div className="flex items-center gap-1 text-[10.5px]">
            <span className="text-[var(--ink-4)] px-1">Sort:</span>
            <button
              type="button"
              onClick={() => setSortBy("name")}
              className={`rounded px-1.5 py-0.5 transition-colors ${
                sortBy === "name"
                  ? "bg-[var(--fill-on)] text-[var(--accent)] font-medium"
                  : "text-[var(--ink-3)] hover:bg-[var(--fill)]"
              }`}
            >
              Name
            </button>
            <button
              type="button"
              onClick={() => setSortBy("usage")}
              className={`rounded px-1.5 py-0.5 transition-colors ${
                sortBy === "usage"
                  ? "bg-[var(--fill-on)] text-[var(--accent)] font-medium"
                  : "text-[var(--ink-3)] hover:bg-[var(--fill)]"
              }`}
            >
              Most used
            </button>
            <button
              type="button"
              onClick={() => setSortBy("recent")}
              className={`rounded px-1.5 py-0.5 transition-colors ${
                sortBy === "recent"
                  ? "bg-[var(--fill-on)] text-[var(--accent)] font-medium"
                  : "text-[var(--ink-3)] hover:bg-[var(--fill)]"
              }`}
            >
              Recent
            </button>
          </div>
        </div>
      )}

      <div
        ref={scrollContainerRef}
        className="scroll flex-1 min-h-0 overflow-y-auto overflow-x-hidden overscroll-contain px-2.5 pb-2 pt-2"
        onContextMenu={(e) =>
          openContextMenu(e, [
            {
              label: "New code…",
              icon: "plus",
              onSelect: () => {
                setShowAdd(true);
                requestAnimationFrame(() => inputRef.current?.focus());
              },
            },
          ])
        }
      >
        {codes.length === 0 ? (
          <div className="px-2 py-6 text-center">
            <Icon
              name="book"
              size={22}
              className="mx-auto opacity-25"
            />
            <p className="hint mt-2.5">
              No codes yet. Add one to start — you'll refine them as you read.
            </p>
          </div>
        ) : filteredAndSortedCodes.length === 0 ? (
          <div className="px-2 py-6 text-center">
            <p className="hint text-[12px]">No codes match “{searchQuery}”.</p>
          </div>
        ) : (
          <>
            {activeDrag && (
              <div
                data-top-level-drop-zone="true"
                data-testid="promote-to-top-zone"
                className={`mb-2 rounded-[9px] border-2 border-dashed py-2 text-center transition-all ${
                  activeDrag.isOverTopLevelZone
                    ? activeDrag.validity.valid
                      ? "border-[var(--accent)] bg-[var(--accent-soft)]"
                      : "border-[var(--danger)] bg-[var(--danger-soft)]"
                    : "border-[var(--border)] opacity-70"
                }`}
              >
                <span className="text-[11.5px] font-medium" style={{ color: "var(--ink-2)" }}>
                  Drop here to promote to top level
                </span>
              </div>
            )}

            <ul className="flex flex-col gap-0.5">
              {filteredAndSortedCodes.map((code) => {
                const selected = selectedCodeIds.includes(code.id);
                const isOpen = expanded === code.id;
                const usage = usageByCode.get(code.id) ?? [];
                const rollup = rollupUsageCounts.get(code.id) ?? code.usage_count;
                const isChild = !!code.parent_id;
                const isDragTarget = activeDrag?.targetCodeId === code.id;
                const isBeingDragged = activeDrag?.draggedCode.id === code.id;

                let dragHighlightBg = undefined;
                let dragHighlightShadow = undefined;
                if (isDragTarget) {
                  if (activeDrag.validity.valid) {
                    dragHighlightBg = "var(--accent-soft)";
                    dragHighlightShadow = "inset 0 0 0 2px var(--accent)";
                  } else {
                    dragHighlightBg = "var(--danger-soft)";
                    dragHighlightShadow = "inset 0 0 0 2px var(--danger)";
                  }
                }

                return (
                  <li
                    key={code.id}
                    data-code-row="true"
                    data-code-id={code.id}
                    tabIndex={0}
                    onKeyDown={(e) => handleRowKeyDown(e, code)}
                    onContextMenu={(e) => openContextMenu(e, codeMenu(code))}
                    className={`group flex flex-col rounded-[11px] transition-all outline-none focus-visible:ring-1 focus-visible:ring-[var(--accent)] ${
                      isChild ? "ml-3.5 pl-1.5 border-l-2 border-[var(--g-rim)]/40" : ""
                    }`}
                    style={{
                      opacity: isBeingDragged ? 0.35 : 1,
                      background:
                        dragHighlightBg ??
                        (selected ? "var(--fill-on)" : "transparent"),
                      boxShadow:
                        dragHighlightShadow ??
                        (selected
                          ? `var(--shadow-1), inset 0 0 0 1px ${code.color}55`
                          : "none"),
                    }}
                    onMouseEnter={(e) => {
                      if (!selected && !isDragTarget)
                        e.currentTarget.style.background = "var(--fill)";
                    }}
                    onMouseLeave={(e) => {
                      if (!selected && !isDragTarget)
                        e.currentTarget.style.background = "transparent";
                    }}
                  >
                  <div className="flex items-start">
                    <button
                      type="button"
                      data-drag-handle="true"
                      onPointerDown={(e) => handlePointerDown(e, code)}
                      onPointerMove={handlePointerMove}
                      onPointerUp={handlePointerUp}
                      onPointerCancel={handlePointerCancel}
                      aria-label={`Reorder "${code.name}" — drag onto another code to nest it`}
                      className={`mt-1.5 ml-1 grid h-6 w-6 shrink-0 place-items-center rounded touch-none select-none transition-colors ${
                        isBeingDragged ? "cursor-grabbing" : "cursor-grab"
                      }`}
                      style={{ color: "var(--ink-2)" }}
                    >
                      <Icon name="grip" size={14} />
                    </button>

                    <button
                      type="button"
                      onClick={() =>
                        setExpanded((cur) => (cur === code.id ? null : code.id))
                      }
                      aria-expanded={isOpen}
                      aria-controls={`code-usage-${code.id}`}
                      className="flex min-w-0 flex-1 items-start gap-2 py-2 pl-1 text-left"
                    >
                      <span
                        className="mt-[5px] grid h-3 w-3 shrink-0 place-items-center rounded-full transition-transform"
                        style={{
                          backgroundColor: code.color,
                          // The check mark draws in currentColor on the solid
                          // fill: keep its contrast at 3:1+, not white.
                          color: textOnSolid(code.color),
                          transform: selected ? "scale(1.15)" : "scale(1)",
                          boxShadow: selected ? `0 0 0 3px ${code.color}33` : "none",
                        }}
                      >
                        {selected && <Icon name="check" size={8} />}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span
                          className="block text-[13px] leading-snug"
                          style={{ fontWeight: selected ? 600 : 450 }}
                        >
                          {code.name}
                        </span>
                        {code.definition && (
                          <span
                            className="mt-0.5 line-clamp-2 block text-[11.5px] leading-snug"
                            style={{ color: "var(--ink-3)" }}
                          >
                            {code.definition}
                          </span>
                        )}
                      </span>
                    </button>

                    {/* Static count reporting & Edit button */}
                    <span className="flex shrink-0 items-center gap-1.5 py-2 pr-2">
                      {code.usage_count > 0 && rollup > code.usage_count ? (
                        <Tooltip content={`Directly on ${code.usage_count} ${code.usage_count === 1 ? "passage" : "passages"}, ${rollup} including sub-codes`}>
                          <span
                            className="font-mono text-[10.5px] tabular-nums"
                            style={{ color: "var(--ink-4)" }}
                          >
                            {code.usage_count} <span className="opacity-60">({rollup})</span>
                          </span>
                        </Tooltip>
                      ) : code.usage_count > 0 ? (
                        <span
                          className="font-mono text-[10.5px] tabular-nums"
                          style={{ color: "var(--ink-4)" }}
                        >
                          {code.usage_count}
                        </span>
                      ) : null}
                      <Tooltip content="Edit, recolour or delete this code">
                        <button
                          type="button"
                          onClick={() => setEditing(code)}
                          aria-label={`Edit ${code.name}`}
                          className="grid h-5 w-5 place-items-center rounded-md opacity-0 transition-opacity focus-visible:opacity-100 group-hover:opacity-100"
                          style={{ color: "var(--ink-3)" }}
                        >
                          <Icon name="dots" size={14} />
                        </button>
                      </Tooltip>
                    </span>
                  </div>

                  {isOpen && (
                    <div
                      id={`code-usage-${code.id}`}
                      className="px-2.5 pb-2"
                    >
                      {usage.length === 0 ? (
                        <p className="hint text-[11.5px]">
                          Not applied anywhere yet. Select text in the
                          transcript to code it.
                        </p>
                      ) : (
                        <ul className="flex flex-col gap-0.5">
                          {usage.map((u) => (
                            <li key={u.key}>
                              <button
                                type="button"
                                onClick={(e) => {
                                  setSelectedSegmentId(u.segmentId, "jump");
                                  if (u.memo) {
                                    clearHoverTimer();
                                    setHoverTarget({
                                      element: e.currentTarget,
                                      code,
                                      quote: u.quote,
                                      memo: u.memo,
                                      coder: u.coder,
                                      activeCoder,
                                      segmentId: u.segmentId,
                                      pinned: true,
                                    });
                                  }
                                }}
                                onMouseEnter={(e) => {
                                  if (!u.memo || hoverTarget?.pinned) return;
                                  clearHoverTimer();
                                  const targetEl = e.currentTarget;
                                  hoverTimerRef.current = window.setTimeout(() => {
                                    setHoverTarget({
                                      element: targetEl,
                                      code,
                                      quote: u.quote,
                                      memo: u.memo,
                                      coder: u.coder,
                                      activeCoder,
                                      segmentId: u.segmentId,
                                      pinned: false,
                                    });
                                  }, 450);
                                }}
                                onMouseLeave={() => {
                                  clearHoverTimer();
                                  if (!hoverTarget?.pinned) {
                                    setHoverTarget(null);
                                  }
                                }}
                                onFocus={(e) => {
                                  if (!u.memo || hoverTarget?.pinned) return;
                                  clearHoverTimer();
                                  const targetEl = e.currentTarget;
                                  hoverTimerRef.current = window.setTimeout(() => {
                                    setHoverTarget({
                                      element: targetEl,
                                      code,
                                      quote: u.quote,
                                      memo: u.memo,
                                      coder: u.coder,
                                      activeCoder,
                                      segmentId: u.segmentId,
                                      pinned: false,
                                    });
                                  }, 450);
                                }}
                                onBlur={() => {
                                  clearHoverTimer();
                                  if (!hoverTarget?.pinned) {
                                    setHoverTarget(null);
                                  }
                                }}
                                className="flex w-full gap-2 rounded-[8px] px-1.5 py-1 text-left transition-colors hover:bg-[var(--fill)]"
                              >
                                <span
                                  className="shrink-0 font-mono text-[11px] tabular-nums"
                                  style={{ color: "var(--ink-4)" }}
                                >
                                  {u.number}
                                </span>
                                <span className="min-w-0 flex-1">
                                  <span
                                    className="line-clamp-2 block font-serif text-[12px] italic leading-snug"
                                    style={{ color: "var(--ink-2)" }}
                                  >
                                    “{u.quote}”
                                  </span>
                                  {u.coder !== activeCoder && (
                                    <span
                                      className="mt-0.5 block text-[10.5px]"
                                      style={{ color: "var(--ink-3)" }}
                                    >
                                      {u.coder}
                                    </span>
                                  )}
                                </span>
                              </button>
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </>
      )}

        {retired.length > 0 && (
          <div className="mt-3 px-1">
            <button
              type="button"
              onClick={() => setShowRetired((v) => !v)}
              className="hint flex w-full items-center gap-1.5 py-1 text-[11px]"
            >
              <Icon
                name={showRetired ? "close" : "eye"}
                size={11}
                className="opacity-60"
              />
              {showRetired ? "Hide" : "Show"} {retired.length} retired
            </button>
            {showRetired && (
              <ul className="mt-1 flex flex-col gap-0.5">
                {retired.map((code) => (
                  <li
                    key={code.id}
                    className="flex items-center gap-2 rounded-[10px] px-2 py-1.5"
                    style={{ background: "var(--fill)" }}
                  >
                    <span
                      className="h-2 w-2 shrink-0 rounded-full opacity-50"
                      style={{ backgroundColor: code.color }}
                    />
                    <span
                      className="min-w-0 flex-1 truncate text-[12px]"
                      style={{ color: "var(--ink-3)" }}
                    >
                      {code.name}
                    </span>
                    <button
                      type="button"
                      onClick={() => restoreCode(code.id)}
                      className="btn btn-ghost btn-sm"
                    >
                      Restore
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </div>

      <NoteHoverCard
        target={hoverTarget}
        onDismiss={() => setHoverTarget(null)}
      />

      <CodeEditorModal
        code={editing}
        allCodes={codes}
        onClose={() => setEditing(null)}
        onSave={updateCode}
        onDelete={deleteCode}
      />

      <div className="p-2.5">
        {showAdd ? (
          <div className="glass-card p-2.5">
            <input
              ref={inputRef}
              value={newCodeName}
              onChange={(e) => setNewCodeName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleAdd();
                if (e.key === "Escape") {
                  setShowAdd(false);
                  setNewCodeName("");
                }
              }}
              placeholder="Name this code"
              className="field field-sm"
              autoFocus
            />
            <p className="hint mt-1.5 text-[11px]">
              Enter adds and keeps going · Esc closes
            </p>
            <div className="mt-2 flex gap-1.5">
              <button
                type="button"
                onClick={handleAdd}
                disabled={!newCodeName.trim()}
                className="btn btn-primary btn-sm flex-1"
              >
                Add code
              </button>
              <button
                type="button"
                onClick={() => {
                  setShowAdd(false);
                  setNewCodeName("");
                }}
                className="btn btn-ghost btn-sm"
              >
                Done
              </button>
            </div>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => setShowAdd(true)}
            className="btn btn-outline btn-block btn-sm"
          >
            <Icon name="plus" size={13} />
            New code
          </button>
        )}
      </div>

      {activeDrag &&
        createPortal(
          <div
            className="pointer-events-none fixed z-[9999] -translate-x-1/2 -translate-y-1/2 select-none"
            style={{ left: activeDrag.pointerX, top: activeDrag.pointerY }}
          >
            <div className="flex items-center gap-1.5 rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 shadow-lg">
              <span
                className="h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: activeDrag.draggedCode.color }}
              />
              <span className="text-[12px] font-medium text-[var(--ink-1)]">
                {activeDrag.draggedCode.name}
              </span>
            </div>
            {!activeDrag.validity.valid && activeDrag.validity.reason && (
              <div className="mt-1 rounded bg-[var(--danger-soft)] px-2 py-0.5 text-[11px] font-medium text-[var(--danger)] shadow-xs whitespace-nowrap">
                {activeDrag.validity.reason}
              </div>
            )}
          </div>,
          document.body,
        )}
    </aside>
  );
}
