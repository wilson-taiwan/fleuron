import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useShallow } from "zustand/react/shallow";
import { useProjectStore } from "../store/project-store";
import { Icon } from "./ui/Icon";
import { modKey } from "../lib/platform";
import { textOnSolid } from "../lib/code-colors";

/**
 * The coding surface, anchored to the text being coded.
 *
 * 🔑 This is now the primary way coding happens. It replaces clicking a code in
 * the codebook, which was never intuitive: a codebook is a reference, and a
 * click on a reference entry should show you the entry rather than silently
 * altering your data. Coding belongs where the evidence is — you read a phrase,
 * you name it, and both happen in one place without your eyes leaving the text.
 *
 * Portalled to `document.body`. Six classes in this app set `backdrop-filter`,
 * which makes an element a containing block for fixed-position descendants, so
 * a bubble rendered inside the transcript panel would be clamped to that column
 * — the same trap already documented in ui/Surfaces and ui/ContextMenu.
 */

export interface BubbleAnchor {
  /**
   * The passage the bubble belongs to, resolved from the DOM each time it is
   * positioned rather than captured once.
   *
   * ⚠️ Holding the element captured at selection time looked equivalent and was
   * not: pressing ↓ moves the selection to the next passage, and the bubble
   * correctly re-described the new one while still floating over the old one,
   * because only its *contents* came from the store. Resolving by id keeps the
   * position and the description reading from the same source. (The panel now
   * also dismisses the bubble when the selection moves passages, so the chase
   * that made this visible cannot happen — but position and description still
   * read from one source.)
   */
  segmentId: string;
  /**
   * The selection rectangle, in coordinates *relative to the passage's `p`*.
   *
   * Stored relative rather than absolute so scrolling stays correct: the
   * viewport rect captured at mouse-up is stale the moment the transcript
   * scrolls, but the offset within the passage never changes. Null means the
   * whole passage is the target, and the bubble sits *under it* — never over
   * the text it is about.
   */
  rel: { top: number; left: number; bottom: number } | null;
}

const GAP = 8;
const WIDTH = 300;

export function SelectionBubble({
  anchor,
  onDismiss,
}: {
  anchor: BubbleAnchor | null;
  onDismiss: () => void;
}) {
  const {
    codes,
    pendingSelection,
    selectedSegmentId,
    segments,
    recentCodeIds,
    currentCodingTarget,
    toggleCodeOnTarget,
    createCodeAndApply,
    openNoteForCoding,
  } = useProjectStore(
    useShallow((s) => ({
      codes: s.codes,
      pendingSelection: s.pendingSelection,
      selectedSegmentId: s.selectedSegmentId,
      segments: s.segments,
      recentCodeIds: s.recentCodeIds,
      currentCodingTarget: s.currentCodingTarget,
      toggleCodeOnTarget: s.toggleCodeOnTarget,
      createCodeAndApply: s.createCodeAndApply,
      openNoteForCoding: s.openNoteForCoding,
    })),
  );

  const ref = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const attachInput = (el: HTMLInputElement | null) => {
    inputRef.current = el;
  };
  const [query, setQuery] = useState("");
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  /**
   * While Shift is held the bubble goes click-through (T01). A Shift+click is
   * a span extension aimed at the passage underneath; a bubble sitting over
   * that point would otherwise swallow the mousedown (collapsing the native
   * selection by default) and the click (never reaching the passage), making
   * extension fail exactly when the bubble is open — which is always, since
   * the drag that summons the bubble precedes every extension.
   */
  const [shiftHeld, setShiftHeld] = useState(false);

  useEffect(() => {
    if (!anchor) {
      setShiftHeld(false);
      return;
    }
    const down = (e: KeyboardEvent) => {
      if (e.key === "Shift") setShiftHeld(true);
    };
    const up = (e: KeyboardEvent) => {
      if (e.key === "Shift") setShiftHeld(false);
    };
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
    };
  }, [anchor]);

  const target = currentCodingTarget();
  const applied = target?.existing?.code_ids ?? [];
  const span =
    pendingSelection && pendingSelection.segmentId === selectedSegmentId
      ? pendingSelection
      : null;
  const segment = segments.find((s) => s.id === selectedSegmentId) ?? null;

  const recentCodes = recentCodeIds
    .map((id) => codes.find((c) => c.id === id))
    .filter((c): c is (typeof codes)[0] => Boolean(c))
    .slice(0, 6);

  // ⌘1–⌘6 keyboard shortcuts to apply recent codes instantly (B6)
  useEffect(() => {
    if (!anchor) return;
    const handleNumKeys = (e: KeyboardEvent) => {
      const isMac =
        typeof navigator !== "undefined" && /Mac/.test(navigator.userAgent);
      const mod = isMac ? e.metaKey : e.ctrlKey;
      if (!mod) return;
      const num = parseInt(e.key, 10);
      if (num >= 1 && num <= 6 && num <= recentCodes.length) {
        e.preventDefault();
        e.stopPropagation();
        const code = recentCodes[num - 1];
        if (code) {
          void toggleCodeOnTarget(code.id);
          setQuery("");
        }
      }
    };

    window.addEventListener("keydown", handleNumKeys, true);
    return () => window.removeEventListener("keydown", handleNumKeys, true);
  }, [anchor, recentCodes, toggleCodeOnTarget]);

  /**
   * Where the bubble goes. The one rule: never cover the text it is about.
   *
   * For a span that means above the selection, flipping below it only when
   * there is genuinely no room above — a bubble below covers the words that
   * follow, which is the context you are reading to decide the code. For a
   * whole passage it means *under the passage*: anchoring "above the passage's
   * bottom edge" was measured hanging over the passage itself and the two
   * above it, which is the exact complaint that motivated this rewrite.
   */
  const place = useCallback(() => {
    if (!anchor) return setPos(null);
    const el = document
      .getElementById(`segment-${selectedSegmentId}`)
      ?.querySelector("p");
    const bubble = ref.current;
    if (!el || !bubble) return;
    const box = el.getBoundingClientRect();
    // Measured, not estimated: the first render goes up hidden precisely so
    // this reads a real height. Guessing 240px placed the bubble wrong by the
    // guess's error on every open — and the guess was never corrected.
    const height = bubble.offsetHeight;
    // A captured rectangle belongs to the passage it was dragged in. Once the
    // selection has moved on, fall back to anchoring under the new passage.
    const rel = anchor.segmentId === selectedSegmentId ? anchor.rel : null;

    let top: number;
    let left: number;
    if (rel) {
      const anchorTop = box.top + rel.top;
      const anchorBottom = box.top + rel.bottom;
      left = box.left + rel.left;
      const above = anchorTop - GAP - height;
      const below = anchorBottom + GAP;
      top =
        above >= 8
          ? above
          : below + height <= window.innerHeight - 8
            ? below
            : Math.max(8, window.innerHeight - height - 8);
    } else {
      left = box.left + 16;
      const below = box.bottom + GAP;
      const above = box.top - GAP - height;
      top =
        below + height <= window.innerHeight - 8
          ? below
          : above >= 8
            ? above
            : Math.max(8, window.innerHeight - height - 8);
    }

    setPos({
      top: Math.max(8, top),
      left: Math.max(8, Math.min(left, window.innerWidth - WIDTH - 8)),
    });
  }, [anchor, selectedSegmentId]);

  // Position before paint, and re-position when the passage scrolls, the
  // window resizes, or the bubble's own height changes — typing in the filter
  // grows and shrinks the matches list, and a bubble that does not track its
  // own height drifts over the text it was placed to avoid.
  useLayoutEffect(() => {
    if (!anchor) return;
    place();
    const scroller = document
      .getElementById(`segment-${selectedSegmentId}`)
      ?.closest(".scroll");
    scroller?.addEventListener("scroll", place, { passive: true });
    window.addEventListener("resize", place);
    const observer = new ResizeObserver(place);
    if (ref.current) observer.observe(ref.current);
    return () => {
      scroller?.removeEventListener("scroll", place);
      window.removeEventListener("resize", place);
      observer.disconnect();
    };
  }, [anchor, selectedSegmentId, place]);

  useEffect(() => {
    if (!anchor) return;
    setQuery("");
    // Focus the filter the moment the bubble opens. Every route that summons
    // it — dragging across words, right-click → Code this passage…, pressing
    // C — is an explicit coding gesture, so typing the code name is always
    // the next thing the coder does; it should cost no extra click. (A plain
    // click on a passage no longer opens the bubble at all: that is how you
    // *move* through the transcript, and a bubble appearing there was the
    // intrusion this component exists to not be.)
    //
    // ⚠️ This used to arm a flag here and focus in the input's ref callback,
    // on the theory that the ref fires exactly when the element mounts. It
    // does — but refs run during the commit, *before* this effect, so on
    // mount the flag was still false and the focus never happened. The rAF
    // is for the opposite hazard: the input is mounted by now, but the
    // browser ignores focus while a drag's mouse-up is still settling.
    const id = requestAnimationFrame(() =>
      inputRef.current?.focus({ preventScroll: true }),
    );
    return () => cancelAnimationFrame(id);
  }, [anchor, selectedSegmentId]);

  useEffect(() => {
    if (!anchor) return;
    // Keyboard dismisses hand focus back to the passage, so Esc lands you
    // exactly where you were and ↑/↓ keep working — the same contract the
    // note panel keeps. Mouse dismisses don't: the pointer owns focus now.
    const dismissKeyboard = () => {
      document.getElementById(`segment-${selectedSegmentId}`)?.focus();
      onDismiss();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      dismissKeyboard();
    };
    const onDown = (e: MouseEvent) => {
      if (ref.current?.contains(e.target as Node)) return;
      // Shift+mousedown is the second half of a Shift+click span extension,
      // not a dismissal: the click handler is about to extend the pending
      // span this bubble belongs to, and dismissing here would clear the
      // anchor before it runs.
      if (e.shiftKey) return;
      onDismiss();
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("mousedown", onDown, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("mousedown", onDown, true);
    };
    // selectedSegmentId is read by dismissKeyboard; anchor identity changes
    // per summon.
  }, [anchor, selectedSegmentId, onDismiss]);

  if (!anchor || !target || !segment) return null;

  const appliedCodes = codes.filter((c) => applied.includes(c.id));
  const q = query.trim().toLowerCase();
  const matches = codes
    .filter((c) => !applied.includes(c.id))
    .filter((c) => !q || c.name.toLowerCase().includes(q))
    .slice(0, 6);
  const exact = codes.some((c) => c.name.toLowerCase() === q);

  return createPortal(
    <div
      ref={ref}
      className="glass-pop anim-rise fixed z-[70] p-2"
      style={
        // First render is present-but-hidden so `place` measures a real
        // height before the bubble has a position to show. Rendering nothing
        // until measured is what made input focus unreliable here.
        pos
          ? {
              top: pos.top,
              left: pos.left,
              width: WIDTH,
              pointerEvents: shiftHeld ? "none" : undefined,
            }
          : {
              top: -9999,
              left: -9999,
              width: WIDTH,
              visibility: "hidden",
            }
      }
      role="dialog"
      aria-label="Code this selection"
      // The bubble must not steal the text selection it is about.
      onMouseDown={(e) => e.stopPropagation()}
    >
      {/* What is about to be coded, quoted back. Without this the bubble is
          ambiguous the moment a passage is selected *and* a phrase inside it
          is — and those are two different codings with different exports. */}
      <div className="flex items-start gap-1.5 px-1 pb-1.5">
        <span className="eyebrow shrink-0 pt-px">
          {span ? "Selection" : `Passage ${segment.segment_index + 1}`}
        </span>
        <span
          className="min-w-0 flex-1 truncate font-serif text-[11.5px] italic"
          style={{ color: "var(--ink-3)" }}
        >
          {span ? `“${span.text}”` : "whole passage"}
        </span>
        <button
          type="button"
          onClick={() => {
            document.getElementById(`segment-${selectedSegmentId}`)?.focus();
            onDismiss();
          }}
          aria-label="Close"
          className="btn btn-ghost btn-icon btn-sm -mr-1 -mt-1 shrink-0"
        >
          <Icon name="close" size={12} />
        </button>
      </div>

      {appliedCodes.length > 0 && (
        <div className="flex flex-wrap gap-1 px-1 pb-2">
          {appliedCodes.map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => void toggleCodeOnTarget(c.id)}
              title={`Remove “${c.name}”`}
              className="flex items-center gap-1 rounded-full px-2 py-0.5 text-[11px] font-medium transition-opacity hover:opacity-75"
              style={{ backgroundColor: c.color, color: textOnSolid(c.color) }}
            >
              {c.name}
              <Icon name="close" size={9} />
            </button>
          ))}
        </div>
      )}

      {recentCodes.length > 0 && !query && (
        <div className="flex flex-wrap items-center gap-1 px-1 pb-2 border-b border-[var(--g-rim)]/40 mb-2">
          <span className="text-[9.5px] uppercase font-semibold tracking-wider text-[var(--ink-4)] mr-0.5 select-none">
            Recent:
          </span>
          {recentCodes.map((c, i) => (
            <button
              key={c.id}
              type="button"
              onClick={() => {
                void toggleCodeOnTarget(c.id);
                setQuery("");
              }}
              title={`Apply ${c.name} (${modKey}${i + 1})`}
              className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium transition-colors bg-[var(--fill)] hover:bg-[var(--fill-hi)] text-[var(--ink)]"
            >
              <span
                className="h-1.5 w-1.5 rounded-full shrink-0"
                style={{ backgroundColor: c.color }}
              />
              <span className="max-w-[70px] truncate">{c.name}</span>
              <kbd className="font-mono text-[9px] opacity-60">
                {modKey}{i + 1}
              </kbd>
            </button>
          ))}
        </div>
      )}

      <input
        ref={attachInput}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key !== "Enter") return;
          e.preventDefault();
          // Enter takes the first match, or creates what was typed. One key for
          // both because at speed the coder does not know or care which case
          // they are in — they know the name they want on this passage.
          if (matches.length > 0) void toggleCodeOnTarget(matches[0].id);
          else if (q) void createCodeAndApply(query);
          setQuery("");
        }}
        placeholder="Code this — type to find or create"
        aria-label="Find or create a code"
        className="field field-sm"
      />

      <ul className="mt-1 flex flex-col">
        {matches.map((c) => (
          <li key={c.id}>
            <button
              type="button"
              onClick={() => {
                void toggleCodeOnTarget(c.id);
                setQuery("");
              }}
              className="flex w-full items-center gap-2 rounded-[8px] px-2 py-1.5 text-left text-[12.5px] transition-colors hover:bg-[var(--fill)]"
            >
              <span
                className="h-2.5 w-2.5 shrink-0 rounded-full"
                style={{ backgroundColor: c.color }}
              />
              <span className="min-w-0 flex-1 truncate">{c.name}</span>
              {c.usage_count > 0 && (
                <span
                  className="shrink-0 font-mono text-[10.5px] tabular-nums"
                  style={{ color: "var(--ink-4)" }}
                >
                  {c.usage_count}
                </span>
              )}
            </button>
          </li>
        ))}

        {q && !exact && (
          <li>
            <button
              type="button"
              onClick={() => {
                void createCodeAndApply(query);
                setQuery("");
              }}
              className="flex w-full items-center gap-2 rounded-[8px] px-2 py-1.5 text-left text-[12.5px] transition-colors hover:bg-[var(--fill)]"
            >
              <Icon name="plus" size={12} />
              {/* One expression: JSX folds the newline-and-indent whitespace
                  around an interpolation into real spaces, which is how this
                  label came out as Create “ name ” with gaps inside the
                  quotes. */}
              <span className="min-w-0 flex-1 truncate">{`Create “${query.trim()}”`}</span>
            </button>
          </li>
        )}

        {codes.length === 0 && !q && (
          <li className="px-2 py-1.5">
            <p className="hint text-[11.5px]">
              No codes yet. Type a name above to make your first one from this
              passage.
            </p>
          </li>
        )}
      </ul>

      {target?.existing && (
        <>
          <div className="divider my-1.5" />
          <button
            type="button"
            onClick={() => {
              if (target.existing) {
                openNoteForCoding(target.existing.id);
              }
            }}
            className="btn btn-ghost btn-sm btn-block"
          >
            <Icon name="note" size={13} />
            {target.existing.memo ? "Edit note" : "Add a note"}
          </button>
        </>
      )}
    </div>,
    document.body,
  );
}
