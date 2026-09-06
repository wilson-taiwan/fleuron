import { isMac, shortcut } from "../lib/platform";
import {
  DATA_BOUNDARY_SUMMARY,
  OFFICIAL_URLS,
} from "./trust-and-permissions";

export type GuideCategory =
  | "getting-started"
  | "sync"
  | "trust"
  | "shortcuts"
  | "troubleshooting"
  | "reflexive-ta";

export interface GuideSection {
  id: string;
  category: GuideCategory;
  title: string;
  whenToUse: string;
  steps: string[];
  expectedResults: string[];
  commonMistakes?: string[];
  relatedSectionIds?: string[];
}

export const GUIDE_CATEGORIES: { id: GuideCategory; label: string }[] = [
  { id: "getting-started", label: "Getting started" },
  { id: "sync", label: "How sync works" },
  { id: "trust", label: "Trust, privacy & permissions" },
  { id: "shortcuts", label: "Keyboard shortcuts" },
  { id: "troubleshooting", label: "Troubleshooting" },
  { id: "reflexive-ta", label: "Reflexive TA" },
];

export const USER_GUIDE_SECTIONS: GuideSection[] = [
  {
    id: "getting-started",
    category: "getting-started",
    title: "Quick reference: getting started",
    whenToUse: "Opening Fleuron, starting a study, or joining an existing study.",
    steps: [
      "Starting a study: Click 'Set up a new study' on the home screen. Name your study and choose a local folder.",
      "Joining a study: Click 'Join with a key' and paste the 8-character study key sent by your colleague.",
      "Importing transcripts: Add participant IDs (e.g. P01, P02) and point Fleuron at your local .docx or .vtt transcript files.",
      "Coding text: Select any text passage in the transcript to open the coding bubble and apply or create codes.",
    ],
    expectedResults: [
      "Your project is stored locally as an SQLite database (project.db).",
      "Codes and coding decisions sync automatically in the background.",
    ],
    relatedSectionIds: ["sync-with-your-coder", "shortcuts", "troubleshooting"],
  },
  {
    id: "sync-with-your-coder",
    category: "sync",
    title: "How sync works (and which fields it excludes)",
    whenToUse: "Understanding collaboration, privacy, and data flow in Fleuron.",
    steps: [
      DATA_BOUNDARY_SUMMARY,
      "Concretely, collaboration shares: your account email; study title and membership; the de-identified study/participant label; interview IDs, segment counts, and content hashes; every codebook field including names, definitions, inclusion/exclusion criteria, examples, colors, and hierarchy; coding attribution and character offsets for each coded passage.",
      "Collaboration sync excludes transcript files/text, quote fields, filenames/paths, and memo fields. Quotes copied into shared codebook fields sync as written. Exports may leave your device through sharing or a synced folder.",
      "Colleagues load identical transcripts locally; matching segment hashes connect their coding to yours automatically.",
      "Sync runs silently in the background when changes settle and whenever you open a study.",
    ],
    expectedResults: [
      "Both coders see shared coding highlights and attribution without sending files back and forth.",
      "Study/participant labels and all codebook text are synced word-for-word — author them de-identified. Fleuron supports a protocol; it does not certify compliance of any kind.",
    ],
    commonMistakes: [
      "Typing slightly different Participant IDs (e.g. P7 vs P07) — IDs decide interview identity, so ensure they match your protocol exactly.",
    ],
    relatedSectionIds: ["trust-privacy-permissions", "getting-started", "troubleshooting"],
  },
  {
    id: "trust-privacy-permissions",
    category: "trust",
    title: "Trust, privacy & permissions",
    whenToUse:
      "Understanding install warnings, what Fleuron accesses, where data lives, and how to verify a build.",
    steps: [
      "Open Trust & permissions from Welcome, Settings, About, or this guide to see warning guidance for your operating system, file access explanations, and build details.",
      "Install warnings: Fleuron is unsigned by Apple/Microsoft publisher identities, so the first launch shows an expected warning on each OS. The Trust panel shows exactly which warnings are expected and which mean stop.",
      "Files & permissions: studies default to ~/Fleuron (macOS) or %USERPROFILE%\\Fleuron (Windows). Choosing a folder in the native picker is the only permission Fleuron asks for; denied access recovers inline without raw error strings.",
      `Verify a build: check version, filename, and the official release page (${OFFICIAL_URLS.releases}); SHA-256 digests and artifact attestations are available under Advanced verification.`,
    ],
    expectedResults: [
      "One authoritative answer for any trust, privacy, or permission question — no contradictory claims elsewhere in the app.",
    ],
    relatedSectionIds: ["sync-with-your-coder", "troubleshooting"],
  },
  {
    id: "shortcuts",
    category: "shortcuts",
    title: "Keyboard shortcuts",
    whenToUse: "Navigating transcripts and coding rapidly without leaving the keyboard.",
    steps: [
      `${shortcut("mod", "1")} to ${shortcut("mod", "6")}: Instantly apply one of your 6 recent codes to the selected text.`,
      `${shortcut("mod", "Z")}: Undo the last coding action or hierarchy drag-to-nest.`,
      `${isMac ? shortcut("mod", "shift", "Z") : shortcut("mod", "Y")}: Redo the undone action.`,
      `${shortcut("mod", "alt", "ArrowRight")}: Nest focused code under the preceding sibling.`,
      `${shortcut("mod", "alt", "ArrowLeft")}: Move focused code back to top level.`,
      "ArrowUp / ArrowDown: Navigate between transcript passages.",
      `${shortcut("mod", "shift", "E")}: Open project export options (CSV & Markdown).`,
      "?: Toggle this User Guide reference.",
    ],
    expectedResults: [
      "Fluent keyboard-driven coding without reaching for the mouse.",
    ],
    relatedSectionIds: ["getting-started", "troubleshooting"],
  },
  {
    id: "working-with-notes",
    category: "getting-started",
    title: "Working with notes: inline editing, saving & recovery",
    whenToUse: "Writing passage notes or interview memos, or finding unfinished work after a crash.",
    steps: [
      "One inline editor: Add or Edit a passage note from the note icon, the coding menu, or the selection bubble — every route opens the same editor docked below its source passage. There is only ever one open at a time.",
      "Explicit save for passage notes: press Save & close (or Cmd/Ctrl+Enter). Collapsing the editor, pressing Escape, switching passage or interview, or filtering keeps the unfinished draft without asking; reopening restores it. Discard asks for confirmation when the draft is dirty.",
      "Interview notes save automatically after a short typing pause. Saved means the current words are committed — a backup alone is never reported as saved.",
      "Local recovery: while a draft is unsaved, Fleuron backs it up on this computer (Unfinished notes, in More actions). Recovery drafts stay on this computer and are not included in study backups or exports. Only backed-up drafts survive a crash — keystrokes still being typed when the app dies are not promised back.",
      "If a note changed elsewhere while your draft was open, you get a side-by-side comparison: save your version, use the saved version, or keep editing. Nothing is overwritten until you choose.",
      "If a note's coding or interview is deleted, the unfinished text stays in Unfinished notes with Copy and Discard — it is never recreated or merged anywhere automatically.",
      "Leaving, closing, or exporting with unfinished notes asks first: Save all, Discard changes, or Cancel (exports offer Export saved notes only instead of discarding). A failed save blocks leaving until it succeeds or you cancel.",
    ],
    expectedResults: [
      "No draft is ever lost to navigation, and no save is ever reported that did not happen.",
    ],
    commonMistakes: [
      "Assuming a draft backed up locally is saved — only Save & close (passage notes) or the Saved indicator (interview notes) commits it.",
    ],
    relatedSectionIds: ["getting-started", "sync-with-your-coder", "troubleshooting"],
  },
  {
    id: "troubleshooting",
    category: "troubleshooting",
    title: "Troubleshooting: near-miss IDs, missing transcripts & conflicts",
    whenToUse: "Diagnosing unlinked transcripts, missing highlights, or sync issues.",
    steps: [
      "Near-miss Participant IDs: If your colleague typed 'P07' and you typed 'P7', Fleuron warns you. Re-name the participant in Interview Settings to match.",
      "Missing transcript file: If a participant has coding from a colleague but no local transcript text, use 'Link transcripts' to point to the local .docx/.vtt file.",
      "Unresolved code highlights: A neutral dotted highlight indicates a code from a colleague that is still pulling or syncing.",
      "Offline work: You can code freely while offline. All changes queue locally and reconcile automatically once reconnected.",
    ],
    expectedResults: [
      "Coding aligns across the team through deterministic content hashes; conflicts surface in the sync sheet instead of silently overwriting anyone's work.",
    ],
    relatedSectionIds: ["sync-with-your-coder", "shortcuts"],
  },
  {
    id: "reflexive-ta",
    category: "reflexive-ta",
    title: "Reflexive thematic analysis principles",
    whenToUse: "Designing codebooks and collaborating organically in Braun & Clarke reflexive TA.",
    steps: [
      "Codes evolve organically: Create, rename, nest, or retire codes as your conceptual understanding deepens.",
      "Overlapping codes: Two coders marking overlapping phrases is normal; hover highlights to view coder attribution.",
      "Two-level hierarchy: Drag sub-codes onto parent categories, use right-click 'Move into ▸', or press ⌘⌥→ to nest.",
    ],
    expectedResults: [
      "A rich, collaborative codebook that captures nuanced interpretations.",
    ],
    relatedSectionIds: ["getting-started", "sync-with-your-coder"],
  },
];

const ALIASES: Record<string, string> = {
  "workspace-overview": "getting-started",
  "add-code": "getting-started",
  "import-vtt-first": "getting-started",
  "apply-codes": "getting-started",
  "keyboard-shortcuts": "shortcuts",
};

export function getGuideSection(id: string): GuideSection | undefined {
  const targetId = ALIASES[id] ?? id;
  return (
    USER_GUIDE_SECTIONS.find((s) => s.id === targetId) ??
    USER_GUIDE_SECTIONS[0]
  );
}

export function searchGuideSections(query: string): GuideSection[] {
  const q = query.trim().toLowerCase();
  if (!q) return USER_GUIDE_SECTIONS;
  return USER_GUIDE_SECTIONS.filter((s) => {
    const text = [
      s.title,
      s.whenToUse,
      ...s.steps,
      ...s.expectedResults,
      ...(s.commonMistakes ?? []),
    ]
      .join(" ")
      .toLowerCase();
    return text.includes(q);
  });
}
