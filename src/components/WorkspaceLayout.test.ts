import { describe, expect, it } from "vitest";
import { fitRails, workspaceColumns } from "./WorkspaceLayout";

/**
 * The transcript is the app's core artifact and the only column whose width is
 * a legibility constraint rather than a preference. These assert the property
 * the layout exists to hold: at the app's own `minWidth: 1024`, the codebook
 * gives way and the reading column keeps its measure.
 *
 * There is exactly one rail now. Passage notes edit inline beneath their
 * source passage, so the memo rail is gone; the stored `memos` width is
 * accepted and ignored for preference compatibility.
 */

const RESIZER = 5;
const TRANSCRIPT_MIN = 512;
const MIN_PANEL = 190;

const transcriptWidth = (r: { codebook: number }, viewport: number) =>
  viewport - r.codebook - RESIZER;

describe("fitRails", () => {
  it("leaves the stored width alone when there is room", () => {
    expect(fitRails(248, 300, 1400)).toEqual({ codebook: 248, memos: 0 });
  });

  it("ignores the legacy memos width instead of reserving space", () => {
    expect(fitRails(248, 300, 1400)).toEqual(fitRails(248, 0, 1400));
    expect(fitRails(248, 480, 1024)).toEqual(fitRails(248, 0, 1024));
  });

  it("keeps the transcript at its floor at the app's minimum window", () => {
    const r = fitRails(248, 300, 1024);
    expect(transcriptWidth(r, 1024)).toBeGreaterThanOrEqual(TRANSCRIPT_MIN);
  });

  it("never shrinks the codebook below MIN_PANEL", () => {
    for (const viewport of [1024, 1100, 1280, 1440]) {
      for (const c of [248, 400, 190]) {
        const r = fitRails(c, 300, viewport);
        expect(r.codebook).toBeGreaterThanOrEqual(MIN_PANEL);
      }
    }
  });

  it("gives up rather than pushing the rail below its floor on a tiny window", () => {
    // narrower than the app supports; the transcript absorbs it instead of a
    // control being driven off-screen
    const r = fitRails(MIN_PANEL, 300, 700);
    expect(r).toEqual({ codebook: MIN_PANEL, memos: 0 });
  });

  it("restores the stored width when the window widens again", () => {
    // With a single rail, an ordinary 248px codebook fits at 1024 untouched;
    // only an over-wide stored width shrinks, and widening restores it.
    expect(fitRails(600, 0, 1024).codebook).toBeLessThan(600);
    expect(fitRails(600, 0, 1600)).toEqual({ codebook: 600, memos: 0 });
    expect(fitRails(248, 300, 1024)).toEqual({ codebook: 248, memos: 0 });
  });
});

describe("workspaceColumns", () => {
  it("renders only the codebook rail when expanded", () => {
    expect(workspaceColumns({ collapsed: false, codebook: 248 })).toBe(
      "248px 5px minmax(0, 1fr)",
    );
  });

  it("replaces the codebook with the slim rail when collapsed", () => {
    expect(workspaceColumns({ collapsed: true, codebook: 248 })).toBe(
      "24px minmax(0, 1fr)",
    );
  });

  it("gives the collapsed width to the transcript, never to a memo rail", () => {
    const collapsed = workspaceColumns({ collapsed: true, codebook: 372 });
    expect(collapsed).not.toContain("372");
    expect(workspaceColumns({ collapsed: false, codebook: 372 })).toContain("372px");
  });
});
