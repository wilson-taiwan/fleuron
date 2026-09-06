import { useCallback, useEffect, useRef, useState } from "react";
import { useAppStore } from "../store/app-store";
import { useProjectStore } from "../store/project-store";
import { hasModKey } from "../lib/platform";
import { Icon } from "./ui/Icon";
import { Tooltip } from "./ui/Tooltip";
import { CodebookPanel } from "./CodebookPanel";
import { TranscriptPanel } from "./TranscriptPanel";
import { InterviewMemoPanel } from "./InterviewMemoPanel";
import { Toolbar } from "./Toolbar";
import { NextStepCoach } from "./NextStepCoach";
import { ActivityLogPanel } from "./ActivityLogPanel";
import { BackupsPanel } from "./BackupsPanel";
import { ProjectFilesPanel } from "./ProjectFilesPanel";
import { CloseProjectModal } from "./CloseProjectModal";
import { ResetWorkspaceModal } from "./ResetWorkspaceModal";
import { ExportDialog } from "./ExportDialog";
import { ContextMenuHost } from "./ui/ContextMenu";

const DEFAULT_CODEBOOK = 248;
const MIN_PANEL = 190;
const MAX_CODEBOOK = 400;
const RESIZER = 5;

/**
 * 🔑 The transcript's floor, and the reason the rails are not simply fixed.
 *
 * `tauri.conf.json` sets `minWidth: 1024`, and at 1024 the fixed rails left the
 * transcript 466px — a ~48-character measure at 15.5px serif, on the app's core
 * artifact, well under the 60–75 long-form reading wants. 512px here leaves
 * ~464px of prose inside `px-6`, which lands at about 60 characters.
 *
 * The rails give this up, not the transcript: a codebook can be read at 190px,
 * a paragraph cannot be read at 45 characters.
 */
const TRANSCRIPT_MIN = 512;

/** Width of the slim expand rail shown while the codebook is collapsed. */
const COLLAPSED_RAIL = 24;

/**
 * Grid columns for the workspace rails (T04).
 *
 * There is exactly one rail now: the codebook. Passage notes edit inline
 * beneath their source passage (one active editor, drafts retained), so the
 * memo rail and its reservation are gone. Collapsed, the codebook and its
 * resizer are replaced by one slim expand rail; the stored codebook width is
 * untouched so expanding restores it. Exported for test: collapsing must
 * give the width to the transcript.
 */
export function workspaceColumns(opts: {
  collapsed: boolean;
  codebook: number;
}): string {
  if (opts.collapsed) {
    return `${COLLAPSED_RAIL}px minmax(0, 1fr)`;
  }
  return `${opts.codebook}px ${RESIZER}px minmax(0, 1fr)`;
}

/**
 * Shrink the codebook toward `MIN_PANEL` when the window cannot hold both it
 * and a readable transcript. Returns the width to *render* — the stored
 * preference is untouched, so widening the window restores exactly what the
 * user chose.
 *
 * Exported for test: this is the one piece of the layout with arithmetic in
 * it, and the failure it guards against (the transcript silently squeezed
 * below a readable measure) is invisible until someone tries to read at 1024.
 *
 * The old `memos` preference is accepted and ignored for compatibility: it
 * stays in stored preferences (no migration that could destroy layout
 * settings) but no longer reserves space.
 */
export function fitRails(
  codebook: number,
  _memosIgnored: number,
  viewport: number,
): { codebook: number; memos: number } {
  void _memosIgnored;
  const overflow = codebook + RESIZER + TRANSCRIPT_MIN - viewport;
  if (overflow <= 0) return { codebook, memos: 0 };

  const slack = Math.max(0, codebook - MIN_PANEL);
  // Already at the floor: the window is narrower than the app claims to
  // support, and the transcript takes what is left rather than pushing a
  // control off-screen.
  if (slack === 0) return { codebook, memos: 0 };

  const take = Math.min(overflow, slack);
  return { codebook: Math.round(codebook - take), memos: 0 };
}

/**
 * The width the rails actually have to divide up, observed on the element
 * rather than read from `window.innerWidth`.
 *
 * ⚠️ A `resize` listener is not equivalent. It misses every path that changes
 * the element's width without resizing the window, and it silently missed a
 * real one during testing: the viewport override used to check this layout
 * changes `innerWidth` without dispatching `resize`, so the rails stayed at
 * their old fitted widths while the `1fr` transcript grew on its own — the
 * layout looked half-updated and the cause was invisible. Observing the box
 * that matters has no such gap.
 */
function useElementWidth(ref: React.RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(() =>
    typeof window === "undefined" ? 1400 : window.innerWidth,
  );
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      setWidth(entry.contentRect.width);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

export function WorkspaceLayout() {
  const panelWidths = useAppStore((s) => s.preferences.panel_widths);
  const setPanelWidths = useAppStore((s) => s.setPanelWidths);
  const codebookCollapsed =
    useAppStore((s) => s.preferences.codebook_collapsed) ?? false;
  const setCodebookCollapsed = useAppStore((s) => s.setCodebookCollapsed);
  const showExportDialog = useProjectStore((s) => s.showExportDialog);
  const setShowExportDialog = useProjectStore((s) => s.setShowExportDialog);
  const [codebookWidth, setCodebookWidth] = useState(
    panelWidths?.codebook ?? DEFAULT_CODEBOOK,
  );
  // The stored memos width is accepted and ignored: it stays in preferences
  // for compatibility, but no memo rail reserves space anymore.

  const gridRef = useRef<HTMLDivElement>(null);
  const gridWidth = useElementWidth(gridRef);
  const rails = fitRails(
    codebookWidth,
    panelWidths?.memos ?? 0,
    gridWidth,
  );
  const dragging = useRef<"codebook" | null>(null);
  const gridWidthRef = useRef(gridWidth);
  gridWidthRef.current = gridWidth;
  const widthsRef = useRef({ codebook: codebookWidth });

  useEffect(() => {
    widthsRef.current = { codebook: codebookWidth };
  }, [codebookWidth]);

  useEffect(() => {
    if (panelWidths) {
      setCodebookWidth(panelWidths.codebook);
    }
  }, [panelWidths]);

  const onMouseMove = useCallback((e: MouseEvent) => {
    // The drag stops where the transcript would drop below TRANSCRIPT_MIN, so
    // the rail can never be dragged into clipping the reading column. Without
    // this the stored width and the rendered width disagree and the panel
    // simply stops following the cursor, which reads as a broken drag.
    if (dragging.current === "codebook") {
      const room = gridWidthRef.current - RESIZER - TRANSCRIPT_MIN;
      const max = Math.min(MAX_CODEBOOK, room);
      const w = Math.min(Math.max(MIN_PANEL, max), Math.max(MIN_PANEL, e.clientX));
      setCodebookWidth(w);
      widthsRef.current.codebook = w;
    }
  }, []);

  const onMouseUp = useCallback(() => {
    if (dragging.current) {
      // Preserve the stored memos width untouched: dropping it here would
      // destroy a layout setting through a no-op migration.
      const { codebook } = widthsRef.current;
      const memos = useAppStore.getState().preferences.panel_widths?.memos;
      void setPanelWidths(codebook, memos ?? 0);
      dragging.current = null;
    }
  }, [setPanelWidths]);

  useEffect(() => {
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, [onMouseMove, onMouseUp]);

  // Codebook collapse toggle (T04): Cmd/Ctrl+B, never while typing or in a
  // dialog. The stored width is untouched while collapsed, so expanding
  // restores exactly what was there.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!hasModKey(e) || e.shiftKey || e.altKey) return;
      if (e.key !== "b" && e.key !== "B") return;
      if (document.querySelector('[role="dialog"]')) return;
      const target = e.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.tagName === "SELECT" ||
          target.isContentEditable)
      ) {
        return;
      }
      e.preventDefault();
      const current =
        useAppStore.getState().preferences.codebook_collapsed ?? false;
      void useAppStore.getState().setCodebookCollapsed(!current);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <Toolbar />

      <div
        ref={gridRef}
        data-testid="workspace-grid"
        data-codebook-collapsed={codebookCollapsed ? "true" : undefined}
        className="relative grid min-h-0 flex-1 overflow-hidden"
          style={
          {
            gridTemplateColumns: workspaceColumns({
              collapsed: codebookCollapsed,
              codebook: rails.codebook,
            }),
            gridTemplateRows: "minmax(0, 1fr)",
            // Published so NextStepCoach can sit over the transcript column
            // alone.
            "--rail-l": codebookCollapsed
              ? `${COLLAPSED_RAIL}px`
              : `${rails.codebook + RESIZER}px`,
            "--rail-r": "0px",
          } as React.CSSProperties
        }
      >
        {codebookCollapsed ? (
          <div
            data-testid="show-codebook-rail"
            className="relative flex h-full w-full items-center justify-center border-r border-[var(--g-rim)] select-none"
          >
            <Tooltip content="Show codebook (Cmd/Ctrl+B)">
              <button
                type="button"
                onClick={() => void setCodebookCollapsed(false)}
                aria-label="Show codebook"
                title="Show codebook (Cmd/Ctrl+B)"
                data-testid="codebook-collapse-toggle"
                className="grid h-6 w-6 place-items-center rounded-md border border-[var(--g-rim)] bg-[var(--surface)] text-[var(--ink-3)] shadow-xs transition-colors hover:bg-[var(--fill)] hover:text-[var(--ink)] cursor-pointer"
              >
                <Icon name="chevronRight" size={13} />
              </button>
            </Tooltip>
          </div>
        ) : (
          <>
            <CodebookPanel />
            <Resizer
              onGrab={() => (dragging.current = "codebook")}
              toggleButton={
                <Tooltip content="Hide codebook (Cmd/Ctrl+B)">
                  <button
                    type="button"
                    onClick={(e) => {
                      e.stopPropagation();
                      void setCodebookCollapsed(true);
                    }}
                    aria-label="Hide codebook"
                    title="Hide codebook (Cmd/Ctrl+B)"
                    data-testid="codebook-collapse-toggle"
                    className="grid h-6 w-6 place-items-center rounded-md border border-[var(--g-rim)] bg-[var(--surface)] text-[var(--ink-3)] shadow-xs transition-colors hover:bg-[var(--fill)] hover:text-[var(--ink)] cursor-pointer"
                  >
                    <Icon name="chevronLeft" size={13} />
                  </button>
                </Tooltip>
              }
            />
          </>
        )}
        <TranscriptPanel />
        <NextStepCoach />
      </div>

      {/* No footer hint bar. It spent 28 permanent pixels repeating five
          shortcuts the coach, the guide, and the context menus already teach
          at the moment they matter; chrome that never changes is chrome that
          stops being read. */}
      {/* No ToastStack here: App.tsx owns the single Notifications region.
          A second host double-announced every failure. */}
      <CloseProjectModal />
      <ResetWorkspaceModal />
      <ExportDialog
        open={showExportDialog}
        onClose={() => setShowExportDialog(false)}
      />
      <ActivityLogPanel />
      <BackupsPanel />
      <ProjectFilesPanel />
      <InterviewMemoPanel />
      <ContextMenuHost />
    </div>
  );
}

function Resizer({
  onGrab,
  toggleButton,
}: {
  onGrab: () => void;
  toggleButton?: React.ReactNode;
}) {
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      onMouseDown={onGrab}
      className="group relative cursor-col-resize select-none"
    >
      <span
        className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 transition-colors group-hover:w-0.5"
        style={{ background: "var(--g-rim)" }}
      />
      {toggleButton && (
        <div
          className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-20 cursor-default"
          onMouseDown={(e) => e.stopPropagation()}
        >
          {toggleButton}
        </div>
      )}
    </div>
  );
}
