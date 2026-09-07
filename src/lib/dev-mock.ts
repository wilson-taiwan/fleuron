/**
 * Browser-only stub for the Tauri IPC bridge.
 *
 * `npm run dev` on its own (no `tauri dev`) has no Rust backend, so every
 * `invoke` throws and the UI can't get past the welcome screen. This installs
 * a fake bridge backed by in-memory fixtures so the whole interface — wizard,
 * workspace, sheets — can be opened and styled in an ordinary browser at
 * Vite's speed.
 *
 * It is a design harness, not a test double: state lives in module scope and
 * dies on reload. Guarded by `import.meta.env.DEV` and by the absence of the
 * real bridge, so it can never reach a packaged build.
 */

import type {
  Code,
  CodedSegment,
  GroupMember,
  Interview,
  ProjectInfo,
  ProjectOpenSnapshot,
  SegmentSpeakerChange,
  TranscriptSegment,
} from "./types";
import { CODE_PALETTE } from "./code-colors";

/** Fixture platform for path shapes: Windows-flavoured only under UA spoof. */
function isWindows(): boolean {
  return typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent);
}

function isStressFixture(): boolean {
  return (
    typeof window !== "undefined" &&
    (window.location.search.includes("fixture=stress") ||
      window.location.hash.includes("fixture=stress"))
  );
}

/**
 * Dev-only fixture: a protocol-1 study with a member who is not ready, so the
 * passive "turns on automatically" state can be exercised. Only reachable with
 * the dev mock (no `tauri` IPC), never in a packaged build.
 */
function isSyncNotReadyFixture(): boolean {
  return (
    typeof window !== "undefined" &&
    (window.location.search.includes("fixture=sync-not-ready") ||
      window.location.hash.includes("fixture=sync-not-ready"))
  );
}

// ── Fault-injection fixtures (E2E plan Task 4) ───────────────────────────────
// Each fixture drives an error/degraded branch of the real UI that the happy
// path never reaches. Reachable only through the browser preview URL.
const FAULT_FIXTURES = ["sync-error", "server-conflict", "auth-error", "slow"] as const;
type FaultFixture = (typeof FAULT_FIXTURES)[number];

function activeFault(): FaultFixture | null {
  if (typeof window === "undefined") return null;
  const text = window.location.search + window.location.hash;
  return FAULT_FIXTURES.find((f) => text.includes(`fixture=${f}`)) ?? null;
}

/** Commands that reject with a network-style failure under fixture=sync-error. */
const SYNC_FAULT_CMDS = new Set([
  "sync_now",
  "sync_deep_verify",
  "sync_repair",
]);

/** Commands that reject with Supabase's invalid-credentials body under fixture=auth-error. */
const AUTH_FAULT_CMDS = new Set(["sync_sign_in", "sync_sign_up"]);

/** Write-path commands artificially delayed under fixture=slow. */
const SLOW_CMDS = new Set([
  "apply_codes",
  "mutate_coding_edge",
  "ensure_code_and_apply",
  "import_segments",
  "export_with_config",
  "create_backup",
  "restore_backup",
]);

const SLOW_FIXTURE_DELAY_MS = 900;

/**
 * One unresolved code-name conflict, shaped exactly like the backend's
 * SyncConflictDetail so the sheet renders its real resolution branch.
 */
function fixtureConflicts() {
  return [
    {
      id: "conflict-fixture-1",
      entity_type: "code" as const,
      entity_label: "Tacit gatekeeping",
      field_name: "name",
      current_value: "Tacit gatekeeping",
      proposed_value: "Tacit gatekeepers",
      proposer_label: "Luci Diaz",
      status: "unresolved" as const,
      created_at: "2026-08-20T10:00:00Z",
    },
  ];
}

function sha256Sync(str: string): string {
  function rightRotate(value: number, amount: number) {
    return (value >>> amount) | (value << (32 - amount));
  }
  let i: number, j: number;
  let result = "";
  const words: number[] = [];

  let hash = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ];

  const k = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0x0bef9a3f, 0xc67178f2,
  ];

  let strUtf8 = unescape(encodeURIComponent(str));
  for (i = 0; i < strUtf8.length; i++) {
    words[i >> 2] |= (strUtf8.charCodeAt(i) & 0xff) << (24 - (i % 4) * 8);
  }
  words[strUtf8.length >> 2] |= 0x80 << (24 - (strUtf8.length % 4) * 8);
  words[(((strUtf8.length + 8) >> 6) << 4) + 15] = strUtf8.length * 8;

  const w = new Array(64);
  for (i = 0; i < words.length; i += 16) {
    let a = hash[0], b = hash[1], c = hash[2], d = hash[3];
    let e = hash[4], f = hash[5], g = hash[6], h = hash[7];

    for (j = 0; j < 64; j++) {
      if (j < 16) {
        w[j] = words[i + j] | 0;
      } else {
        const gamma0 = rightRotate(w[j - 15], 7) ^ rightRotate(w[j - 15], 18) ^ (w[j - 15] >>> 3);
        const gamma1 = rightRotate(w[j - 2], 17) ^ rightRotate(w[j - 2], 19) ^ (w[j - 2] >>> 10);
        w[j] = (w[j - 16] + gamma0 + w[j - 7] + gamma1) | 0;
      }
      const s1 = rightRotate(e, 6) ^ rightRotate(e, 11) ^ rightRotate(e, 25);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (h + s1 + ch + k[j] + w[j]) | 0;
      const s0 = rightRotate(a, 2) ^ rightRotate(a, 13) ^ rightRotate(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (s0 + maj) | 0;

      h = g;
      g = f;
      f = e;
      e = (d + temp1) | 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) | 0;
    }

    hash[0] = (hash[0] + a) | 0;
    hash[1] = (hash[1] + b) | 0;
    hash[2] = (hash[2] + c) | 0;
    hash[3] = (hash[3] + d) | 0;
    hash[4] = (hash[4] + e) | 0;
    hash[5] = (hash[5] + f) | 0;
    hash[6] = (hash[6] + g) | 0;
    hash[7] = (hash[7] + h) | 0;
  }

  for (i = 0; i < 8; i++) {
    for (j = 3; j >= 0; j--) {
      const byte = (hash[i] >> (j * 8)) & 0xff;
      result += (byte < 16 ? "0" : "") + byte.toString(16);
    }
  }
  return result;
}

/**
 * Synthetic demo interview. Written for this fixture — not a transcript, not a
 * paraphrase of one, and not derived from any real interview or participant.
 *
 * Two rules if you edit it. Keep the domain mundane: this file is public, and a
 * demo that reads like real research data invites the question of whose it is.
 * And keep the coded-segment offsets below in step — they index into
 * TRANSCRIPT[2][1] by character, so changing that turn silently moves the
 * highlight the demo is meant to show.
 */
const TRANSCRIPT: [string, string][] = [
  [
    "Interviewer",
    "Thanks for making the time. Could you start by telling me how you first got involved with the garden?",
  ],
  [
    "P04",
    "There is a waiting list, officially. Four years, they told me on the phone. I had a plot in about six weeks, because I happened to help somebody carry compost.",
  ],
  [
    "P04",
    "That is how most of it works, honestly. Nothing is written down anywhere, and the people who have been here longest simply know who gets which plot and when it comes free.",
  ],
  [
    "Interviewer",
    "Nothing written down — can you say more about what that means day to day?",
  ],
  [
    "P04",
    "It means you learn by getting it wrong. I strimmed the path behind my shed in my first month and three separate people told me we do not cut that side until August. Nobody had thought to mention it.",
  ],
  [
    "P04",
    "By the second season I was the one telling somebody else. That was the strange part. I had become one of the people who knows, and I still could not have pointed you at a rule.",
  ],
  [
    "Interviewer",
    "What happens when someone does not pick it up?",
  ],
  [
    "P04",
    "They drift off. The plot goes over, they stop turning up to work days, and eventually there is a letter. But the letter is never really about the weeds.",
  ],
  [
    "P04",
    "We lost a family last spring who I think were never actually let in. They were here eight months and I am not sure anyone learned their names.",
  ],
  [
    "Interviewer",
    "How did that sit with you?",
  ],
  [
    "P04",
    "Badly. Because I remember arriving and being handed a mug of tea inside ten minutes, and that was the entire difference. Somebody decided I was worth the tea.",
  ],
  [
    "P04",
    "We call ourselves a community, and mostly we are. But there is a door, and not everyone gets shown where it is.",
  ],
];

const standardInterviews: Interview[] = [
  {
    id: "iv-1",
    participant_label: "P04-2026-03-11",
    interview_date: "2026-03-11",
    modality: "zoom",
    diagnosis_notes: null,
    interviewers: ["Ada Lovelace"],
    hub_memo:
      "Strong through-line about who gets let in and who does not. Compare with P02 — same site, very different arrival.",
    audio_path: null,
    segment_count: TRANSCRIPT.length,
    remote_segment_count: null,
  },
  {
    id: "iv-2",
    participant_label: "P07-2026-04-02",
    interview_date: "2026-04-02",
    modality: "zoom",
    diagnosis_notes: null,
    interviewers: ["Ada Lovelace"],
    hub_memo: null,
    audio_path: null,
    segment_count: 0,
    remote_segment_count: 214,
  },
];

const standardSegments: TranscriptSegment[] = TRANSCRIPT.map(([speaker, text], i) => ({
  id: `seg-${i}`,
  interview_id: "iv-1",
  segment_index: i,
  speaker,
  timestamp_start: `00:${String(2 + i * 3).padStart(2, "0")}:${String((i * 17) % 60).padStart(2, "0")}.000`,
  timestamp_end: null,
  text,
  block_id: i === 2 || i === 5 ? `q00${i}` : null,
  section_tag: null,
}));

const standardCodes: Code[] = [
  {
    id: "c1",
    name: "Unwritten rules",
    definition: "Norms that govern the site but exist nowhere in writing.",
    inclusion_criteria: null,
    exclusion_criteria: null,
    example: null,
    parent_id: null,
    color: "#8a6410",
    sort_order: 0,
    is_retired: false,
    usage_count: 1,
  },
  {
    id: "c2",
    name: "Learning by transgression",
    definition: "Discovering a rule by breaking it and being corrected.",
    inclusion_criteria: null,
    exclusion_criteria: null,
    example: null,
    parent_id: null,
    color: "#1f7a5e",
    sort_order: 1,
    is_retired: false,
    usage_count: 1,
  },
  {
    id: "c3",
    name: "Waiting list",
    definition: null,
    inclusion_criteria: null,
    exclusion_criteria: null,
    example: null,
    parent_id: null,
    color: "#b03a34",
    sort_order: 2,
    is_retired: false,
    usage_count: 0,
  },
  {
    id: "c4",
    name: "Threshold of belonging",
    definition: "Informal admission granted by existing members, separate from formal membership.",
    inclusion_criteria: null,
    exclusion_criteria: null,
    example: null,
    parent_id: null,
    color: "#3d6f96",
    sort_order: 3,
    is_retired: false,
    usage_count: 1,
  },
];

const standardCodedSegments: CodedSegment[] = [
  {
    id: "cs1",
    interview_id: "iv-1",
    segment_id: "seg-2",
    code_ids: ["c1"],
    coder_name: "Ada Lovelace",
    memo: "States 'nothing is written down' outright, then names who holds it anyway.",
    char_start: null,
    char_end: null,
    quote_text: TRANSCRIPT[2][1],
    block_id: "q002",
    timestamp_start: standardSegments[2].timestamp_start,
    participant_label: "P04-2026-03-11",
  },
  {
    id: "cs2",
    interview_id: "iv-1",
    segment_id: "seg-5",
    code_ids: ["c2"],
    coder_name: "Luci Diaz",
    memo: "Becomes an enforcer of a rule she still cannot articulate.",
    char_start: null,
    char_end: null,
    quote_text: TRANSCRIPT[5][1],
    block_id: "q005",
    timestamp_start: standardSegments[5].timestamp_start,
    participant_label: "P04-2026-03-11",
  },
  {
    id: "cs3",
    interview_id: "iv-1",
    segment_id: "seg-2",
    code_ids: ["c4"],
    coder_name: "Luci Diaz",
    memo: null,
    char_start: 78,
    char_end: 115,
    quote_text: TRANSCRIPT[2][1].slice(78, 115),
    block_id: "q002",
    timestamp_start: standardSegments[2].timestamp_start,
    participant_label: "P04-2026-03-11",
  },
];

/**
 * 320-segment corpus for layout-stress testing. Synthetic, on the same terms as
 * TRANSCRIPT above — this is the second copy, and the one easier to forget.
 */
function generateStressFixture() {
  const speakers = ["Interviewer", "P04", "Interviewer", "P04", "P04"];
  const snippets = [
    "Could you walk me through what actually happens on a work day at the site?",
    "It starts at nine, though the same four people have been there since eight. There is a list on the shed door, and there is the real list, which is whatever those four settled over the first pot of tea.",
    "Do newer members pick that distinction up quickly, or does it take a while to become visible?",
    "Most never see it at all. They take the shed list for the whole arrangement, and then they cannot work out why their requests keep going nowhere.",
    "When the water is rationed in July the allocation is supposed to be strictly per plot, and in practice it follows who has helped whom.",
    "My old neighbour assumed the committee decided everything, because that is what the constitution says on paper.",
    "In reality most of it is settled well before any meeting, in the ten minutes while the kettle boils.",
    "Can you describe how a new person is actually shown the ropes, given that none of it is written down?",
    "Somebody adopts you, or nobody does. There is no middle version of it, and no one will ever tell you which one has happened to you.",
    "I was told for years that the waiting list was strictly chronological, so I assumed everyone had arrived the same way I did.",
  ];

  const stressSegments: TranscriptSegment[] = [];
  for (let i = 0; i < 320; i++) {
    const spk = speakers[i % speakers.length];
    const snip = snippets[i % snippets.length];
    const text =
      i % 4 === 0
        ? `${snip} Furthermore, when the committee moves the work-day rota at short notice, the informal arrangements collapse and everyone falls back on asking whoever has been here longest.`
        : snip;
    const min = Math.floor(i / 2);
    const sec = (i * 30) % 60;
    stressSegments.push({
      id: `seg-${i}`,
      interview_id: "iv-1",
      segment_index: i,
      speaker: spk,
      timestamp_start: `00:${String(min).padStart(2, "0")}:${String(sec).padStart(2, "0")}.000`,
      timestamp_end: null,
      text,
      block_id: i % 10 === 0 ? `blk-${i}` : null,
      section_tag: null,
    });
  }

  const stressCodes: Code[] = [];
  for (let i = 1; i <= 60; i++) {
    stressCodes.push({
      id: `c${i}`,
      name: `Analytic Concept ${i} — Extended Descriptive Strategy Title`,
      definition: `Detailed inclusion rules, boundary criteria, and theoretical grounding for concept ${i} within reflexive thematic analysis.`,
      inclusion_criteria: `Use when explicit evidence of an informal norm or an access barrier for dimension ${i} is present.`,
      exclusion_criteria: null,
      example: null,
      parent_id: i > 10 && i % 5 === 0 ? `c${i - 5}` : null,
      color: CODE_PALETTE[(i - 1) % CODE_PALETTE.length],
      sort_order: i - 1,
      is_retired: false,
      usage_count: i <= 20 ? 8 : 2,
    });
  }

  const stressCoded: CodedSegment[] = [];
  let noteCounter = 0;
  for (let i = 0; i < 160; i++) {
    const segIdx = (i * 2) % 320;
    const seg = stressSegments[segIdx];
    const coder = i % 3 === 0 ? "Luci Diaz" : "Ada Lovelace";
    const codeId1 = `c${(i % 55) + 1}`;
    const codeId2 = `c${((i + 7) % 55) + 1}`;
    const codeIds = i % 2 === 0 ? [codeId1, codeId2] : [codeId1];
    const isSpan = i % 3 !== 0 && seg.text.length > 30;
    const char_start = isSpan ? 10 : null;
    const char_end = isSpan ? Math.min(seg.text.length, 50) : null;
    const hasNote = i % 3 === 0 || noteCounter < 45;
    if (hasNote) noteCounter++;
    const memo = hasNote
      ? `Analytic note ${noteCounter}: Participant articulates the cost of conscious regulation in passage ${segIdx + 1}.`
      : null;

    stressCoded.push({
      id: `cs-stress-${i}`,
      interview_id: "iv-1",
      segment_id: seg.id,
      code_ids: codeIds,
      coder_name: coder,
      memo,
      char_start,
      char_end,
      quote_text: isSpan && char_start != null && char_end != null ? seg.text.slice(char_start, char_end) : seg.text,
      block_id: seg.block_id,
      timestamp_start: seg.timestamp_start,
      participant_label: "P04-2026-03-11",
    });
  }

  const stressInterviews: Interview[] = [
    {
      id: "iv-1",
      participant_label: "P04-2026-03-11",
      interview_date: "2026-03-11",
      modality: "zoom",
      diagnosis_notes: null,
      interviewers: ["Ada Lovelace", "Luci Diaz"],
      hub_memo: "Extended stress test dataset for performance and layout validation.",
      audio_path: null,
      segment_count: 320,
      remote_segment_count: null,
    },
  ];

  return {
    interviews: stressInterviews,
    segments: stressSegments,
    codes: stressCodes,
    codedSegments: stressCoded,
  };
}

let interviews: Interview[] = [];
let segments: TranscriptSegment[] = [];
let codes: Code[] = [];
let codedSegments: CodedSegment[] = [];
let reviewedSegmentIds = new Set<string>();

function resetFixture() {
  const f = isStressFixture()
    ? generateStressFixture()
    : {
        interviews: standardInterviews.map((x) => ({ ...x })),
        segments: standardSegments.map((x) => ({ ...x })),
        codes: standardCodes.map((x) => ({ ...x })),
        codedSegments: standardCodedSegments.map((x) => ({ ...x, code_ids: [...x.code_ids] })),
      };
  interviews = f.interviews;
  segments = f.segments;
  codes = f.codes;
  codedSegments = f.codedSegments;
  reviewedSegmentIds = new Set();
  return f;
}

resetFixture();

// --- note drafts: workspace identity + recovery mirror -----------------------
// Mirrors the backend's checked-write contract (epoch/project-key validation,
// revision CAS, tombstones, conditional cleanup) so the browser preview
// exercises the real draft lifecycle without a Rust backend.
let mockProjectKey: string | null = "mock-project-key-1";
let mockEpoch: string | null = "mock-epoch-1";
let mockDraftSeq = 0;

// Like the backend's register_project: the key is stable per resolved study
// path (clones stay separate), and every open re-establishes it — a close
// must never leave a later open keyless while carrying a fresh epoch.
const mockProjectKeys = new Map<string, string>([
  ["/Users/demo/Drive/sample-study.fleuron", "mock-project-key-1"],
]);

function mockKeyForStudy(path: string): string {
  let key = mockProjectKeys.get(path);
  if (!key) {
    key = `mock-project-key-${mockProjectKeys.size + 1}`;
    mockProjectKeys.set(path, key);
  }
  mockProjectKey = key;
  return key;
}

function rotateMockWorkspace() {
  mockEpoch = `mock-epoch-${Math.random().toString(36).slice(2, 8)}`;
}

type MockDraft = {
  draft_id: string;
  project_key: string;
  kind: string;
  target_id: string;
  interview_id: string;
  coder_name: string | null;
  base_text: string;
  draft_text: string;
  revision: number;
  participant_label: string;
  segment_id: string | null;
  segment_index: number | null;
  char_start: number | null;
  char_end: number | null;
  quote_text: string | null;
  updated_at: string;
  discarded: boolean;
  project_title: string | null;
};

const mockDrafts = new Map<string, MockDraft>();

function mockLiveMemo(kind: string, targetId: string): string | null {
  if (noteFault("missing") && kind === "coding") return null;
  if (kind === "coding") {
    const coding = codedSegments.find((c) => c.id === targetId);
    return coding ? (coding.memo ?? "") : null;
  }
  const interview = interviews.find((i) => i.id === targetId);
  return interview ? (interview.hub_memo ?? "") : null;
}

function mockActiveDraft(projectKey: string, kind: string, targetId: string) {
  for (const draft of mockDrafts.values()) {
    if (
      !draft.discarded &&
      draft.project_key === projectKey &&
      draft.kind === kind &&
      draft.target_id === targetId
    ) {
      return draft;
    }
  }
  return null;
}

/** Explicit dev-mock fault fixtures for the note lifecycle surfaces. */
function noteFault(name: "recovery" | "commit" | "slow" | "changed" | "missing"): boolean {
  if (typeof window === "undefined") return false;
  const text = window.location.search + window.location.hash;
  if (name === "slow") return text.includes("fixture=note-commit-slow");
  return text.includes(`fixture=note-${name}-fail`) || text.includes(`fixture=note-${name}`);
}

function checkMockWorkspace(projectKey: string, epoch: string): boolean {
  return (
    mockProjectKey !== null &&
    mockEpoch !== null &&
    projectKey === mockProjectKey &&
    epoch === mockEpoch
  );
}

/** Snapshots the browser harness pretends to hold. Empty at first, like a real project's. */
const mockBackups: {
  path: string;
  file_name: string;
  size_bytes: number;
  created_at: string;
  reason: "manual" | "automatic" | "pre-restore";
  note: string | null;
  app_version: string;
  schema_version: number;
  project_title: string;
  codes: number;
  interviews: number;
  segments: number;
  coded_segments: number;
}[] = [];

let openProject: Record<string, unknown> | null = null;

function buildSnapshot(): ProjectOpenSnapshot {
  const activeInterviewId = interviews.length > 0 ? interviews[0].id : null;
  const ivSegments = segments.filter((s) => s.interview_id === activeInterviewId);
  const ivCoded = codedSegments.filter((c) => c.interview_id === activeInterviewId);
  const selectedSegmentId = isStressFixture()
    ? "seg-180"
    : ivSegments.length > 0
      ? ivSegments[0].id
      : null;

  return {
    project: (openProject ?? DEMO) as unknown as ProjectInfo,
    codes: [...codes],
    interviews: [...interviews],
    workspace: {
      active_interview_id: activeInterviewId,
      selected_segment_id: selectedSegmentId,
      active_coder: "Ada Lovelace",
    },
    active_interview_id: activeInterviewId,
    selected_segment_id: selectedSegmentId,
    segments: ivSegments,
    coded_segments: ivCoded,
    total_coded_count: codedSegments.length,
    recent_code_ids: codes.slice(0, 6).map((c) => c.id),
    reviewed_segment_ids: [],
    diagnostics: {
      schema_version: 4,
      counts: {
        codes: codes.length,
        interviews: interviews.length,
        segments: ivSegments.length,
        coded_segments: ivCoded.length,
      },
      timings_ms: {
        connection: 2,
        schema: 5,
        snapshot_queries: 12,
        total: 19,
      },
    },
    project_key: mockProjectKey,
    workspace_epoch: mockEpoch,
  };
}

export type IpcLogEntry = {
  cmd: string;
  at: string;
  ok: boolean;
  err?: string;
};

const IPC_RING_MAX = 200;
export const ipcRing: IpcLogEntry[] = [];

function recordIpc(cmd: string, ok: boolean, err?: string) {
  ipcRing.push({ cmd, at: new Date().toISOString(), ok, err });
  if (ipcRing.length > IPC_RING_MAX) ipcRing.shift();
}

export function clearIpcRing() {
  ipcRing.length = 0;
}

/**
 * Sync state for the preview harness.
 *
 * `configured: true` so the chip is visible on load — the preview exists to
 * exercise surfaces, and a control that hides until a preference is written
 * would never be seen. Signed in by default so grouped projects adopt the roster
 * name on open; sign-out flows can still call sync_sign_out in the mock.
 */
const mockSync = {
  configured: true,
  signedIn: true,
  pending: 3,
  joined: true,
  lastSyncedAt: null as string | null,
  groupKey: "ABCD-1234",
  // Auto-activation is stateful: sync_v2_activate flips this to 2, and the
  // status + readiness handlers read it. The not-ready fixture keeps it on 1.
  protocol: 1 as 1 | 2,
  allReady: !isSyncNotReadyFixture(),
  generationSuffix: null as string | null,
  head: 0,
  // The roster the group sheet renders. "Ada" (no membership) is the
  // coding-before-the-group-existed case, kept visible on purpose.
  members: [
    {
      coderName: "Ada Lovelace",
      userId: "user-1",
      role: "admin",
      joinedAt: "2026-06-30T18:04:00Z",
      lastActiveAt: new Date(Date.now() - 36e5 * 5).toISOString(),
      codedCount: 47,
      isYou: true,
    },
    {
      coderName: "Luci Diaz",
      userId: "user-2",
      role: "coder",
      joinedAt: "2026-07-02T16:40:00Z",
      lastActiveAt: new Date(Date.now() - 36e5 * 30).toISOString(),
      codedCount: 31,
      isYou: false,
    },
    {
      coderName: "Ada",
      userId: undefined,
      role: undefined,
      joinedAt: null,
      lastActiveAt: new Date(Date.now() - 36e5 * 52).toISOString(),
      codedCount: 9,
      isYou: false,
    },
  ] as GroupMember[],
};
let workspace = {
  active_interview_id: "iv-1" as string | null,
  selected_segment_id: "seg-2" as string | null,
  active_coder: "Ada Lovelace" as string | null,
};
let prefs = {
  reopen_last_project: false,
  last_guide_section_id: null as string | null,
  panel_widths: null,
  coach_dismissed: false,
  merge_same_speaker: true,
  theme: "light",
  // Empty on purpose: the browser preview should exercise the "Who's coding?"
  // prompt, which is what a teammate opening a shared study actually hits.
  coder_identities: {} as Record<string, string>,
  // Set, so the preview shows the collapsed "already configured" server row
  // rather than the first-run form. Clear them to see the setup path.
  sync_url: "https://example-project.supabase.co" as string | null,
  sync_anon_key: "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.preview" as string | null,
  transcript_zoom: null as number | null,
  codebook_collapsed: false,
  speaker_redaction: {} as Record<string, boolean>,
};
let mockUpdateStatus = {
  phase: "idle",
  currentVersion: "browser preview",
  targetVersion: null as string | null,
  downloadedBytes: 0,
  totalBytes: null as number | null,
  lastCheckedAt: null as string | null,
  syncPreflightOutcome: null as string | null,
  failure: null as { stage: string; retryable: boolean; message: string } | null,
};

const DEMO = {
  path: "/Users/demo/Drive/sample-study.fleuron",
  title: "Sample Study",
  methodology: "reflexive-ta",
  coders: ["Ada Lovelace"],
  last_saved_by: "Luci Diaz",
  last_saved_at: new Date(Date.now() - 36e5 * 30).toISOString(),
  checked_out_by: "Luci Diaz" as string | null,
  checked_out_at: new Date(Date.now() - 36e5 * 30).toISOString() as string | null,
};

function mockExport(
  targetDir: string = "/Users/demo/Desktop",
  config?: { items?: string[] },
) {
  const items = config?.items ?? ["report-html", "coded-segments", "framework-matrix"];
  const files: string[] = ["export-manifest.json"];
  if (items.includes("report-html")) files.push("report.html");
  if (items.includes("coded-segments")) files.push("coded-segments.csv");
  if (items.includes("framework-matrix")) files.push("framework-matrix.csv");

  const folderName = `sample-study-export-${new Date().toISOString().slice(0, 10)}`;
  return {
    exports_dir: `${targetDir}/${folderName}`,
    coded_segment_count: codedSegments.length,
    interview_file_count: 0,
    unresolved_conflict_count: 0,
    files,
    exported_at: new Date().toISOString(),
    exported_by: "Ada Lovelace",
  };
}

async function handle(cmd: string, args: Record<string, unknown>): Promise<unknown> {
  // fixture=server-conflict keeps the study on protocol 2 so the sheet's
  // conflict branch (which requires v2) actually renders. Applied lazily
  // because mockSync is declared below module scope's resetFixture() call.
  if (activeFault() === "server-conflict" && mockSync.protocol !== 2) {
    mockSync.protocol = 2;
  }
  switch (cmd) {
    // --- dialog / opener plugins -----------------------------------------
    case "plugin:dialog|open":
      return (args?.options as { directory?: boolean })?.directory
        ? "/Users/demo/Drive"
        : "/Users/demo/Downloads/zoom-transcript.vtt";
    case "plugin:dialog|save":
      return "/Users/demo/Desktop/export.csv";
    case "plugin:dialog|confirm":
    case "plugin:dialog|ask":
      return true;
    case "plugin:opener|open_path":
    case "plugin:opener|open_url":
      return null;
    case "plugin:event|listen":
    case "plugin:event|unlisten":
      return 1;

    // --- project lifecycle -----------------------------------------------
    case "create_project":
      resetFixture();
      openProject = DEMO;
      mockKeyForStudy((args.input as { project_name?: string })?.project_name ?? DEMO.path as string);
      rotateMockWorkspace();
      return DEMO;
    case "open_project":
      resetFixture();
      openProject = DEMO;
      mockKeyForStudy(String(args.path ?? DEMO.path));
      rotateMockWorkspace();
      return buildSnapshot();
    case "get_project_info":
      return openProject ?? DEMO;
    case "get_live_workspace_snapshot": {
      const requested = args.activeInterviewId as string | null;
      const activeInterviewId = interviews.some((interview) => interview.id === requested)
        ? requested
        : interviews[0]?.id ?? null;
      const activeSegments = segments.filter(
        (segment) => segment.interview_id === activeInterviewId,
      );
      const activeCoded = codedSegments.filter(
        (coding) => coding.interview_id === activeInterviewId,
      );
      const fixtureConflictsActive =
        activeFault() === "server-conflict" ? fixtureConflicts() : [];
      return {
        project: openProject ?? DEMO,
        interviews: [...interviews],
        codes: codes.filter((code) => !code.is_retired),
        retired_codes: codes.filter((code) => code.is_retired),
        active_interview_id: activeInterviewId,
        selected_segment_id: activeSegments[0]?.id ?? null,
        segments: activeSegments,
        coded_segments: activeCoded,
        pending_coded_count: 0,
        coded_count: codedSegments.length,
        conflicts: fixtureConflictsActive,
        sync_status: {
          protocol: 1,
          generation: null,
          local_sequence: 0,
          observed_head: 0,
          outbox_count: 0,
          blocked_count: 0,
          unresolved_conflict_count: fixtureConflictsActive.length,
        },
        local_revision: 0,
        reviewed_segment_ids: [],
        project_key: mockProjectKey,
        workspace_epoch: mockEpoch,
      };
    }
    case "adopt_project_coder": {
      const to = String(args.to ?? "").trim();
      const target = openProject ?? DEMO;
      if (to) {
        target.coders = [to];
      }
      return target;
    }
    case "close_project":
      openProject = null;
      mockProjectKey = null;
      mockEpoch = null;
      return null;

    // --- content ----------------------------------------------------------
    // Backups. Enough shape for the panel to be laid out and styled; the
    // snapshots are fiction and restoring one is a no-op, because the browser
    // harness has no database to swap.
    case "create_backup": {
      const made = {
        path: `/fake/backups/fleuron-manual-${mockBackups.length}.fleuronbak`,
        file_name: `fleuron-manual-${mockBackups.length}.fleuronbak`,
        size_bytes: 190_000 + mockBackups.length * 1_500,
        created_at: new Date().toISOString(),
        reason: "manual" as const,
        note: (args?.note as string | null) ?? null,
        app_version: "0.12.1",
        schema_version: 3,
        project_title: "Sample Study",
        codes: codes.length,
        interviews: interviews.length,
        segments: segments.length,
        coded_segments: codedSegments.length,
      };
      mockBackups.unshift(made);
      return made;
    }

    case "list_backups":
      return mockBackups;

    case "pending_coded_count":
      return 0;

    case "delete_backup": {
      const path = args?.backupPath as string;
      const at = mockBackups.findIndex((b) => b.path === path);
      if (at >= 0) mockBackups.splice(at, 1);
      return undefined;
    }

    case "inspect_backup":
    case "import_backup":
      return mockBackups[0];

    case "restore_backup":
      mockKeyForStudy(DEMO.path as string);
      rotateMockWorkspace();
      return {
        restored_from: args?.backupPath as string,
        restored: mockBackups[0],
        safety_backup_path: "/fake/backups/fleuron-pre-restore.fleuronbak",
      };

    case "save_local_copy":
      return {
        path: `${args?.destinationDir ?? "/fake/backups"}/copy.fleuronbak`,
        timestamp: new Date().toISOString(),
        size_bytes: 1024,
        segment_count: segments.length,
        code_count: codes.length,
        coded_segment_count: 5,
        memo_count: 2,
      };

    case "restore_study_backup":
      return `/fake/projects/restored-${Date.now()}`;

    case "list_codes":
      return codes.filter((c) => !c.is_retired);
    case "list_retired_codes":
      return codes.filter((c) => c.is_retired);
    case "create_code": {
      const input = args.input as { name: string; color?: string; definition?: string };
      const created = {
        ...codes[0],
        id: `c${codes.length + 1}`,
        name: input.name,
        definition: input.definition ?? null,
        color: input.color ?? "#8a6410",
        sort_order: codes.length,
        usage_count: 0,
      };
      codes.push(created);
      return created;
    }
    case "ensure_code_and_apply": {
      const input = args.input as {
        name: string;
        color?: string;
        interview_id: string;
        segment_id: string;
        coder_name: string;
        char_start?: number;
        char_end?: number;
      };
      const normalizedName = input.name.trim().toLocaleLowerCase();
      let code = codes.find(
        (candidate) =>
          !candidate.is_retired &&
          candidate.name.trim().toLocaleLowerCase() === normalizedName,
      );
      const created = !code;
      if (!code) {
        code = {
          id: `c${codes.length + 1}`,
          name: input.name.trim(),
          definition: null,
          inclusion_criteria: null,
          exclusion_criteria: null,
          example: null,
          parent_id: null,
          color: input.color ?? "#8a6410",
          sort_order: codes.length,
          is_retired: false,
          usage_count: 0,
        };
        codes.push(code);
      }
      const charStart = input.char_start ?? null;
      const charEnd = input.char_end ?? null;
      let coded = codedSegments.find(
        (candidate) =>
          candidate.segment_id === input.segment_id &&
          candidate.coder_name === input.coder_name &&
          candidate.char_start === charStart &&
          candidate.char_end === charEnd,
      );
      const changed = !coded?.code_ids.includes(code.id);
      if (coded) {
        if (changed) coded.code_ids = [...coded.code_ids, code.id];
      } else {
        const segment = segments.find((candidate) => candidate.id === input.segment_id);
        coded = {
          id: `cs${codedSegments.length + 1}`,
          interview_id: input.interview_id,
          segment_id: input.segment_id,
          code_ids: [code.id],
          coder_name: input.coder_name,
          memo: null,
          char_start: charStart,
          char_end: charEnd,
          quote_text:
            segment && charStart != null && charEnd != null
              ? segment.text.slice(charStart, charEnd)
              : (segment?.text ?? ""),
          block_id: segment?.block_id ?? null,
          timestamp_start: segment?.timestamp_start ?? "",
          participant_label: "P04-2026-03-11",
        };
        codedSegments.push(coded);
      }
      return {
        code,
        coded_segment: { ...coded, code_ids: [...coded.code_ids] },
        created,
        changed_edges: changed
          ? [{
              interview_id: input.interview_id,
              segment_id: input.segment_id,
              code_id: code.id,
              char_start: charStart,
              char_end: charEnd,
              present: true,
            }]
          : [],
      };
    }
    case "update_code": {
      const input = args.input as {
        id: string;
        name: string;
        definition?: string | null;
        inclusion_criteria?: string | null;
        exclusion_criteria?: string | null;
        example?: string | null;
        parent_id?: string | null;
        color: string;
      };
      const code = codes.find((c) => c.id === input.id);
      if (!code) return null;
      code.name = input.name;
      code.definition = input.definition ?? null;
      code.inclusion_criteria = input.inclusion_criteria ?? null;
      code.exclusion_criteria = input.exclusion_criteria ?? null;
      code.example = input.example ?? null;
      code.parent_id = input.parent_id ?? null;
      code.color = input.color;
      return code;
    }
    case "delete_code": {
      const codeId = args.codeId as string;
      const mode = args.mode as "retire" | "purge";
      const code = codes.find((c) => c.id === codeId);
      if (!code) return null;
      if (mode === "retire") {
        code.is_retired = true;
        return { mode, code_name: code.name, segments_updated: 0, coded_segments_removed: 0 };
      }
      let segments_updated = 0;
      let coded_segments_removed = 0;
      for (let i = codedSegments.length - 1; i >= 0; i--) {
        const cs = codedSegments[i];
        if (!cs.code_ids.includes(codeId)) continue;
        segments_updated++;
        cs.code_ids = cs.code_ids.filter((id) => id !== codeId);
        if (cs.code_ids.length === 0) {
          codedSegments.splice(i, 1);
          coded_segments_removed++;
        }
      }
      codes.splice(codes.indexOf(code), 1);
      return { mode, code_name: code.name, segments_updated, coded_segments_removed };
    }
    case "restore_code": {
      const code = codes.find((c) => c.id === args.codeId);
      if (code) code.is_retired = false;
      return null;
    }
    case "list_interviews":
      return interviews;
    case "create_interview": {
      const input = args.input as { participant_label: string; interview_date?: string };
      const created = {
        ...interviews[1],
        id: `iv-${interviews.length + 1}`,
        participant_label: input.participant_label,
        interview_date: input.interview_date ?? null,
        hub_memo: null,
        segment_count: 0,
        remote_segment_count: null,
      };
      interviews.push(created);
      return created;
    }
    case "update_interview": {
      const input = args.input as {
        id: string;
        participant_label: string;
        interview_date?: string | null;
      };
      const interview = interviews.find((i) => i.id === input.id);
      if (!interview) return null;
      interview.participant_label = input.participant_label;
      interview.interview_date = input.interview_date ?? null;
      return interview;
    }
    case "interview_delete_impact": {
      const interview = interviews.find((i) => i.id === args.interviewId);
      return {
        participant_label: interview?.participant_label ?? "",
        segment_count: interview?.segment_count ?? 0,
        remote_segment_count: null,
        coded_segment_count: codedSegments.filter(
          (c) => c.interview_id === args.interviewId,
        ).length,
        has_hub_memo: !!interview?.hub_memo,
      };
    }
    case "delete_interview": {
      const idx = interviews.findIndex((i) => i.id === args.interviewId);
      if (idx >= 0) interviews.splice(idx, 1);
      return null;
    }
    case "get_segments":
      return args.interviewId === "iv-1" ? segments.map((s) => ({ ...s })) : [];
    case "list_segment_reviews":
      return segments
        .filter((s) => reviewedSegmentIds.has(s.id))
        .map((s) => s.id);
    case "set_segment_reviewed": {
      const segmentId = args.segmentId as string;
      const reviewed = Boolean(args.reviewed);
      if (reviewed) {
        reviewedSegmentIds.add(segmentId);
      } else {
        reviewedSegmentIds.delete(segmentId);
      }
      return null;
    }
    case "set_segment_speaker": {
      const segmentId = args.segmentId as string;
      const newSpeaker = String(args.newSpeaker ?? "").trim();
      const includeFollowing = Boolean(args.includeFollowing);
      if (!newSpeaker) throw new Error("Speaker name cannot be empty");
      const targetIndex = segments.findIndex((s) => s.id === segmentId);
      if (targetIndex === -1) throw new Error(`Segment not found: ${segmentId}`);
      const target = segments[targetIndex];
      const oldSpeaker = target.speaker;
      const changes: SegmentSpeakerChange[] = [];
      if (includeFollowing) {
        for (let i = targetIndex; i < segments.length; i++) {
          if (segments[i].speaker === oldSpeaker) {
            changes.push({
              segment_id: segments[i].id,
              old_speaker: oldSpeaker,
              new_speaker: newSpeaker,
            });
            segments[i].speaker = newSpeaker;
          }
        }
      } else {
        changes.push({
          segment_id: target.id,
          old_speaker: oldSpeaker,
          new_speaker: newSpeaker,
        });
        target.speaker = newSpeaker;
      }
      return changes;
    }
    case "restore_segment_speakers": {
      const changes = (args.changes as SegmentSpeakerChange[]) || [];
      for (const change of changes) {
        const seg = segments.find((s) => s.id === change.segment_id);
        if (seg) {
          seg.speaker = change.old_speaker;
        }
      }
      return null;
    }
    case "get_interview_speakers": {
      const interviewId = (args.interviewId ?? args.interview_id) as string;
      const ivSegments = segments.filter((s) => s.interview_id === interviewId);
      const speakerCounts = new Map<string, number>();
      for (const seg of ivSegments) {
        const spk = seg.speaker.trim() || "Unknown";
        speakerCounts.set(spk, (speakerCounts.get(spk) ?? 0) + 1);
      }
      return Array.from(speakerCounts.entries()).map(([speaker, turn_count]) => ({
        speaker,
        turn_count,
      }));
    }
    case "rename_interview_speaker": {
      const input = (args.input ?? args) as {
        interview_id: string;
        old_speaker: string;
        new_speaker: string;
        expected_count?: number;
      };
      const changes: SegmentSpeakerChange[] = [];
      for (const s of segments) {
        if (s.interview_id === input.interview_id && s.speaker === input.old_speaker) {
          changes.push({
            segment_id: s.id,
            old_speaker: input.old_speaker,
            new_speaker: input.new_speaker,
          });
          s.speaker = input.new_speaker;
        }
      }
      return changes;
    }
    case "undo_rename_interview_speaker": {
      const input = (args.input ?? args) as {
        changes: SegmentSpeakerChange[];
      };
      const changes = input.changes || [];
      for (const change of changes) {
        const seg = segments.find((s) => s.id === change.segment_id);
        if (seg) {
          seg.speaker = change.old_speaker;
        }
      }
      return null;
    }
    case "list_coded_segments":
      return args.interviewId && args.interviewId !== "iv-1"
        ? []
        : codedSegments.map((c) => ({ ...c, code_ids: [...c.code_ids] }));
    case "apply_codes": {
      // 🔑 Mirrors the real upsert: one row per (passage, coder, span) — the
      // row key includes the char offsets, NULL-safe. It previously keyed on
      // (passage, coder) alone and hard-coded the offsets to null, which made
      // span coding unobservable in this harness: every highlight landed as a
      // whole-passage row, and a second span on one passage overwrote the
      // first's codes. The browser preview is where UI claims get verified;
      // it has to tell the truth about spans.
      const input = args.input as {
        interview_id: string;
        segment_id: string;
        code_ids: string[];
        coder_name: string;
        memo?: string;
        char_start?: number;
        char_end?: number;
      };
      const charStart = input.char_start ?? null;
      const charEnd = input.char_end ?? null;
      const existing = codedSegments.find(
        (c) =>
          c.segment_id === input.segment_id &&
          c.coder_name === input.coder_name &&
          c.char_start === charStart &&
          c.char_end === charEnd,
      );
      if (existing) {
        existing.code_ids = [...input.code_ids];
        existing.memo = input.memo ?? null;
        return existing;
      }
      const seg = segments.find((s) => s.id === input.segment_id);
      const created: CodedSegment = {
        id: `cs${codedSegments.length + 1}`,
        interview_id: input.interview_id,
        segment_id: input.segment_id,
        code_ids: [...input.code_ids],
        coder_name: input.coder_name,
        memo: input.memo ?? null,
        char_start: charStart,
        char_end: charEnd,
        quote_text:
          seg && charStart != null && charEnd != null
            ? seg.text.slice(charStart, charEnd)
            : (seg?.text ?? ""),
        block_id: seg?.block_id ?? null,
        timestamp_start: seg?.timestamp_start ?? "",
        participant_label: "P04-2026-03-11",
      };
      codedSegments.push(created);
      return created;
    }
    case "mutate_coding_edge": {
      const input = args.input as {
        interview_id: string;
        segment_id: string;
        code_id: string;
        coder_name: string;
        char_start?: number;
        char_end?: number;
        present: boolean;
      };
      const charStart = input.char_start ?? null;
      const charEnd = input.char_end ?? null;
      const index = codedSegments.findIndex(
        (candidate) =>
          candidate.segment_id === input.segment_id &&
          candidate.coder_name === input.coder_name &&
          candidate.char_start === charStart &&
          candidate.char_end === charEnd,
      );
      let coded = index >= 0 ? codedSegments[index] : undefined;
      if (input.present && !coded) {
        const segment = segments.find((candidate) => candidate.id === input.segment_id);
        coded = {
          id: `cs${codedSegments.length + 1}`,
          interview_id: input.interview_id,
          segment_id: input.segment_id,
          code_ids: [],
          coder_name: input.coder_name,
          memo: null,
          char_start: charStart,
          char_end: charEnd,
          quote_text:
            segment && charStart != null && charEnd != null
              ? segment.text.slice(charStart, charEnd)
              : (segment?.text ?? ""),
          block_id: segment?.block_id ?? null,
          timestamp_start: segment?.timestamp_start ?? "",
          participant_label: "P04-2026-03-11",
        };
        codedSegments.push(coded);
      }
      const wasPresent = !!coded?.code_ids.includes(input.code_id);
      if (coded && input.present && !wasPresent) {
        coded.code_ids.push(input.code_id);
      }
      if (coded && !input.present && wasPresent) {
        coded.code_ids = coded.code_ids.filter((id) => id !== input.code_id);
      }
      if (coded && coded.code_ids.length === 0) {
        const removeIndex = codedSegments.indexOf(coded);
        if (removeIndex >= 0) codedSegments.splice(removeIndex, 1);
        coded = undefined;
      }
      return {
        coded_segment: coded ? { ...coded, code_ids: [...coded.code_ids] } : null,
        changed_edge: {
          interview_id: input.interview_id,
          segment_id: input.segment_id,
          code_id: input.code_id,
          char_start: charStart,
          char_end: charEnd,
          present: input.present,
        },
      };
    }
    case "patch_coding_memo": {
      if (noteFault("commit")) {
        throw new Error("Simulated write rejection (fixture=note-commit-fail).");
      }
      const input = args.input as { coded_segment_id: string; memo?: string };
      const coded = codedSegments.find((candidate) => candidate.id === input.coded_segment_id);
      if (!coded) throw new Error("That note's coding is no longer available.");
      coded.memo = input.memo ?? null;
      return { ...coded, code_ids: [...coded.code_ids] };
    }
    case "delete_coded_segment": {
      const idx = codedSegments.findIndex((c) => c.id === args.codedSegmentId);
      if (idx >= 0) codedSegments.splice(idx, 1);
      return null;
    }
    case "import_segments":
      return segments.length;
    case "update_hub_memo": {
      if (noteFault("commit")) {
        throw new Error("Simulated write rejection (fixture=note-commit-fail).");
      }
      const interview = interviews.find((i) => i.id === args.interviewId);
      if (!interview) throw new Error("That interview is no longer available.");
      interview.hub_memo = args.memo as string;
      return null;
    }
    case "save_workspace_state":
      workspace = args.workspace as typeof workspace;
      return null;
    case "get_workspace_state":
      return workspace;

    // --- note drafts: local recovery mirror --------------------------------
    case "note_recovery_status":
      return noteFault("recovery")
        ? {
            available: false,
            error: "Draft recovery is unavailable (fixture=note-recovery-fail). Editing still works in memory.",
          }
        : { available: true, error: null };
    case "list_note_drafts":
      return [...mockDrafts.values()]
        .filter((d) => !d.discarded)
        .sort((a, b) => (a.updated_at < b.updated_at ? 1 : -1))
        .map((d) => ({ ...d }));
    case "begin_note_draft": {
      if (noteFault("recovery")) {
        throw new Error("Draft recovery is unavailable (fixture=note-recovery-fail). Editing still works in memory.");
      }
      const input = args.input as {
        project_key: string;
        epoch: string;
        kind: string;
        target_id: string;
        interview_id: string;
        coder_name?: string | null;
        participant_label: string;
        segment_id?: string | null;
        segment_index?: number | null;
        char_start?: number | null;
        char_end?: number | null;
        quote_text?: string | null;
      };
      if (!checkMockWorkspace(input.project_key, input.epoch)) {
        return { status: "stale-workspace" };
      }
      const live = mockLiveMemo(input.kind, input.target_id);
      if (live === null) return { status: "missing-target" };
      const existing = mockActiveDraft(input.project_key, input.kind, input.target_id);
      if (existing) {
        if (input.participant_label && existing.participant_label !== input.participant_label) {
          existing.participant_label = input.participant_label;
        }
        return { status: "active", record: { ...existing }, committed_text: live };
      }
      for (const [id, draft] of mockDrafts) {
        if (
          draft.discarded &&
          draft.project_key === input.project_key &&
          draft.kind === input.kind &&
          draft.target_id === input.target_id
        ) {
          mockDrafts.delete(id);
        }
      }
      const record: MockDraft = {
        draft_id: `mock-draft-${mockDrafts.size + 1}-${Date.now()}`,
        project_key: input.project_key,
        kind: input.kind,
        target_id: input.target_id,
        interview_id: input.interview_id,
        coder_name: input.coder_name ?? null,
        base_text: live,
        draft_text: live,
        revision: 0,
        participant_label: input.participant_label,
        segment_id: input.segment_id ?? null,
        segment_index: input.segment_index ?? null,
        char_start: input.char_start ?? null,
        char_end: input.char_end ?? null,
        quote_text: input.quote_text ?? null,
        updated_at: new Date().toISOString(),
        discarded: false,
        project_title: "Sample Study",
      };
      mockDrafts.set(record.draft_id, record);
      mockDraftSeq += 1;
      return { status: "active", record: { ...record }, committed_text: live };
    }
    case "put_note_draft": {
      if (noteFault("recovery")) {
        throw new Error("Draft recovery is unavailable (fixture=note-recovery-fail). Editing still works in memory.");
      }
      const input = args.input as {
        project_key: string;
        epoch: string;
        draft_id: string;
        expected_revision: number;
        draft_text: string;
      };
      if (!checkMockWorkspace(input.project_key, input.epoch)) {
        return { status: "stale-workspace" };
      }
      const draft = mockDrafts.get(input.draft_id);
      if (!draft || draft.discarded) return { status: "stale-generation" };
      if (draft.revision !== input.expected_revision) {
        return { status: "revision-mismatch", record: { ...draft } };
      }
      draft.draft_text = input.draft_text;
      draft.revision += 1;
      draft.updated_at = new Date().toISOString();
      mockDraftSeq += 1;
      return { status: "stored", record: { ...draft } };
    }
    case "discard_note_draft": {
      if (noteFault("recovery")) {
        throw new Error("Draft recovery is unavailable (fixture=note-recovery-fail). Editing still works in memory.");
      }
      const input = args.input as { draft_id: string };
      const draft = mockDrafts.get(input.draft_id);
      if (!draft) throw new Error("That unfinished note is no longer in local recovery.");
      draft.discarded = true;
      draft.draft_text = "";
      draft.base_text = "";
      draft.updated_at = new Date().toISOString();
      mockDraftSeq += 1;
      return { draft_id: draft.draft_id, revision: draft.revision };
    }
    case "save_note_draft": {
      if (noteFault("commit")) {
        throw new Error("Simulated write rejection (fixture=note-commit-fail).");
      }
      if (noteFault("slow")) {
        await new Promise((r) => setTimeout(r, 1500));
      }
      const input = args.input as {
        project_key: string;
        epoch: string;
        draft_id: string;
        revision: number;
        kind: string;
        target_id: string;
        expected_saved_text: string;
        draft_text: string;
      };
      if (!checkMockWorkspace(input.project_key, input.epoch)) {
        return { status: "stale-workspace" };
      }
      const memoryOnly = !input.draft_id;
      const draft = memoryOnly ? null : mockDrafts.get(input.draft_id);
      if (!memoryOnly && (!draft || draft.discarded)) {
        return { status: "stale-generation" };
      }
      let live = mockLiveMemo(input.kind, input.target_id);
      if (live === null) return { status: "missing-target" };
      if (noteFault("changed")) {
        // Something committed under the draft: force the comparison branch.
        live = "Changed under you while you were typing (fixture=note-changed).";
        if (input.kind === "coding") {
          const coding = codedSegments.find((c) => c.id === input.target_id);
          if (coding) coding.memo = live;
        } else {
          const interview = interviews.find((i) => i.id === input.target_id);
          if (interview) interview.hub_memo = live;
        }
      }
      if (live !== input.expected_saved_text) {
        return { status: "conflict", current_text: live };
      }
      if (input.kind === "coding") {
        const coding = codedSegments.find((c) => c.id === input.target_id);
        if (coding) coding.memo = input.draft_text || null;
      } else {
        const interview = interviews.find((i) => i.id === input.target_id);
        if (interview) interview.hub_memo = input.draft_text;
      }
      mockDraftSeq += 1;
      let recoveryCleared = false;
      if (draft) {
        if (draft.revision > input.revision) draft.base_text = input.draft_text;
        if (draft.revision === input.revision) {
          mockDrafts.delete(draft.draft_id);
          recoveryCleared = true;
        }
      }
      return {
        status: "saved",
        draft_id: input.draft_id,
        revision: input.revision,
        committed_text: input.draft_text,
        recovery_cleared: recoveryCleared,
      };
    }
    case "resolve_note_draft_target": {
      const draft = mockDrafts.get(args.draftId as string);
      if (!draft || draft.discarded) {
        throw new Error("That unfinished note is no longer in local recovery.");
      }
      if (!openProject || !mockProjectKey) {
        return { status: "missing", draft_id: draft.draft_id, reason: "no-project" };
      }
      if (draft.project_key !== mockProjectKey) {
        return { status: "missing", draft_id: draft.draft_id, reason: "other-project" };
      }
      const live = mockLiveMemo(draft.kind, draft.target_id);
      if (live === null) {
        return { status: "missing", draft_id: draft.draft_id, reason: "deleted" };
      }
      if (draft.kind === "coding") {
        const coding = codedSegments.find((c) => c.id === draft.target_id);
        return {
          status: "live-coding",
          draft_id: draft.draft_id,
          interview_id: coding?.interview_id ?? draft.interview_id,
          participant_label: coding?.participant_label ?? draft.participant_label,
          committed_text: live,
        };
      }
      const interview = interviews.find((i) => i.id === draft.target_id);
      return {
        status: "live-interview",
        draft_id: draft.draft_id,
        participant_label: interview?.participant_label ?? draft.participant_label,
        committed_text: live,
      };
    }
    case "set_recovery_root_for_selftest":
      return null;
    case "approve_update_departure":
      return {
        token: `mock-approval-${Date.now()}`,
        project_key: mockProjectKey,
        epoch: mockEpoch,
        draft_write_seq: mockDraftSeq,
      };
    case "complete_note_departure":
      return {
        intent_id: args.intentId as string,
        replayed: (args.approved as boolean) ? "close" : "cancelled",
      };
    case "clear_workspace":
      return {
        cleared_coded_segments: 2,
        cleared_block_ids: 2,
        cleared_hub_memos: 1,
        cleared_activity_log: false,
        scope: "all",
      };

    // --- exports, files, activity -----------------------------------------
    case "export_with_config":
      return mockExport(
        (args?.targetDir as string) || "/Users/demo/Desktop",
        args?.config as { items?: string[] },
      );
    case "export_project":
      return mockExport();
    case "consume_pending_open":
      // Nothing is ever double-clicked into the browser harness. Returning an
      // empty queue keeps the mount path identical to the real one.
      return [];
    case "get_projects_library_dir":
      // Fixture paths never embed a real username; the separator follows the
      // harness platform so wizard previews look native on both.
      return isWindows() ? "C:\\Users\\you\\Fleuron" : "/Users/you/Fleuron";
    case "library_sync_warning":
      // The default library is local in the demo — the interesting case is
      // reachable by picking a path with a provider name in it.
      return null;
    case "cloud_provider_for_path": {
      // Lets the browser preview exercise the wizard's advisory without a real
      // Box mount: any path mentioning a provider triggers it.
      const p = String(args.path ?? "");
      const hit = ["Box", "Dropbox", "OneDrive", "Google Drive", "iCloud"].find((n) =>
        p.toLowerCase().includes(n.toLowerCase().replace(" ", "")) ||
        p.toLowerCase().includes(n.toLowerCase()),
      );
      return hit ?? null;
    }
    case "list_project_files":
      return [
        {
          path: `${DEMO.path}/exports/coded-segments.csv`,
          relative_path: "exports/coded-segments.csv",
          name: "coded-segments.csv",
          size: 4821,
          modified_at: new Date().toISOString(),
          kind: "csv",
        },
        {
          path: `${DEMO.path}/exports/P04-2026-03-11.md`,
          relative_path: "exports/P04-2026-03-11.md",
          name: "P04-2026-03-11.md",
          size: 9310,
          modified_at: new Date().toISOString(),
          kind: "markdown",
        },
      ];
    case "hash_candidate_segments": {
      const segments = (args.segments as Array<{ index: number; text: string }>) || [];
      if (segments.length === 0) return null;
      let str = "";
      for (const s of segments) {
        str += `${s.index}\0${s.text.trim().split(/\s+/).join(" ")}\0`;
      }
      return sha256Sync(str);
    }
    case "scan_transcript_folder":
      return [
        {
          path: "/Users/demo/Transcripts/P04.vtt",
          name: "P04.vtt",
          raw_text: "WEBVTT\n\n00:00:01.000 --> 00:00:04.000\nP04: Hello",
        },
      ];
    case "copy_project_file":
      return null;
    case "adopt_project_title": {
      const title = (args.title as string) || "Sample Study";
      return {
        path: DEMO.path,
        title,
        coders: ["Ada Lovelace"],
        last_saved_by: "Ada Lovelace",
        last_saved_at: new Date().toISOString(),
        created_at: new Date().toISOString(),
        schema_version: 1,
      };
    }
    case "list_cached_memberships":
      return {
        memberships: mockSync.joined
          ? [
              {
                projectId: "proj-1",
                title: "Sample Study",
                coderName: "Ada Lovelace",
                members: ["Ada Lovelace", "Luci Diaz"],
                role: "admin",
              },
            ]
          : [],
        cachedAt: new Date().toISOString(),
      };

    // Sync. Stateful across calls so the preview can be driven through the real
    // sequence — configure, sign in, sync — rather than only rendering one
    // frozen state. `mockSync` lives at module scope below.
    case "sync_status":
      return {
        configured: mockSync.configured,
        signedIn: mockSync.signedIn,
        signedInEmail: mockSync.signedIn ? "ada@example.com" : null,
        // The preview shows the shipping shape: a build that knows its server,
        // so the join screen asks for a group key rather than a pasted block.
        serverPreset: true,
        projectId: "7f3a1c92-4d5e-4b8a-9c31-0e6f2a8d4b17",
        inGroup: mockSync.joined,
        isGroupAdmin: true,
        lastSyncedAt: mockSync.lastSyncedAt,
        pendingChanges: mockSync.pending,
        neverSynced: false,
        realtimeConnected: mockSync.joined,
        realtimeHealth: mockSync.joined ? "connected" : "off",
        serverSchemaVersion: 10,
        requiredServerSchema: 10,
        pendingUnbindOperation: null,
        coordinatorRunning: false,
        coordinatorRerunRequested: false,
        coordinatorBackoffAttempt: 0,
        coordinatorLastTrigger: null,
        oldestOutboxAgeSeconds: null,
        protocol: mockSync.protocol,
        generationSuffix: mockSync.protocol === 2 ? mockSync.generationSuffix : null,
        localSequence: 0,
        observedHead: mockSync.protocol === 2 ? mockSync.head : 0,
        outboxCount: mockSync.pending,
        blockedOutboxCount: 0,
        unresolvedConflictCount:
          activeFault() === "server-conflict" ? fixtureConflicts().length : 0,
        lastRealtimeAt: null,
        lastSuccessAt: mockSync.lastSyncedAt,
        sequenceLagAgeSeconds: null,
        deviceIdSuffix: "00000001",
      };
    case "is_selftest":
      return false;
    case "sync_sign_in":
      mockSync.signedIn = true;
      return null;
    case "sync_sign_out":
      mockSync.signedIn = false;
      return null;
    case "sync_restore_session":
      return false;
    case "sync_sign_up":
      mockSync.signedIn = true;
      return true;
    case "sync_request_password_reset":
      return null;
    case "sync_complete_password_reset":
      mockSync.signedIn = true;
      return null;
    case "sync_redeem_invite":
      mockSync.joined = true;
      return { projectId: "proj-1", coderName: "Sam" };
    case "sync_list_memberships":
      return mockSync.joined
        ? [
            {
              projectId: "proj-1",
              title: "Sample Study",
              coderName: "Ada Lovelace",
              members: ["Ada Lovelace", "Luci Diaz"],
              role: "admin",
            },
          ]
        : [];
    case "sync_join_project":
      mockSync.joined = true;
      return null;
    case "sync_create_project":
      mockSync.joined = true;
      return { projectId: "proj-1", groupKey: mockSync.groupKey };
    case "sync_join_group": {
      mockSync.joined = true;
      const name = (args.coderName as string) || "Sam";
      if (!mockSync.members.some((m) => m.coderName === name)) {
        mockSync.members.push({
          coderName: name,
          userId: "user-joined",
          role: "coder",
          joinedAt: new Date().toISOString(),
          lastActiveAt: null,
          codedCount: 0,
          isYou: false,
        });
      }
      return {
        projectId: "proj-1",
        title: "Sample Study",
        coderName: name,
        created: true,
      };
    }
    case "sync_group_info":
      return {
        title: "Sample Study",
        groupKey: mockSync.groupKey,
        members: mockSync.members,
      };
    case "sync_v2_readiness":
      return {
        protocol: mockSync.protocol,
        generationSuffix: mockSync.protocol === 2 ? mockSync.generationSuffix : null,
        head: mockSync.protocol === 2 ? mockSync.head : 0,
        members: mockSync.members
          .filter((member) => member.userId)
          .map((member, index) => ({
            userId: member.userId,
            coderName: member.coderName,
            role: member.role ?? "coder",
            ready: mockSync.allReady,
            readyAt: mockSync.allReady || member.role === "admin"
              ? new Date().toISOString()
              : null,
            lastDeviceIdSuffix: mockSync.allReady || index === 0 ? "00000001" : null,
          })),
      };
    case "sync_v2_activate":
      mockSync.protocol = 2;
      mockSync.generationSuffix = "00000002";
      mockSync.head = 0;
      return {
        protocol: 2,
        generationSuffix: "00000002",
        head: 0,
        legacyActorRows: 0,
      };
    case "sync_v2_resolve_conflict":
      return null;
    case "list_sync_conflicts":
      return activeFault() === "server-conflict" ? fixtureConflicts() : [];
    case "sync_reset_group_key":
      mockSync.groupKey = "X7KM-9P2Q";
      return mockSync.groupKey;
    case "sync_set_member_role": {
      const userId = args.userId as string;
      const role = args.role as string;
      const member = mockSync.members.find((m) => m.userId === userId);
      if (member) member.role = role;
      return null;
    }
    case "sync_remove_member": {
      const userId = args.userId as string;
      mockSync.members = mockSync.members.filter((m) => m.userId !== userId);
      return null;
    }
    case "sync_delete_group": {
      mockSync.joined = false;
      mockSync.members = [];
      return null;
    }
    case "sync_leave_group": {
      mockSync.joined = false;
      mockSync.members = mockSync.members.filter((m) => !m.isYou);
      return null;
    }
    case "sync_detach_local": {
      mockSync.joined = false;
      return null;
    }
    case "project_deletion_summary": {
      return {
        interview_count: 3,
        coded_segment_count: 42,
        memo_count: 5,
      };
    }
    case "delete_project_folder": {
      return null;
    }
    case "inspect_join_target": {
      const parentDir = (args.parentDir as string) || "";
      const slug = (args.slug as string) || "";
      return {
        verdict: "available",
        suggested_name: slug,
        suggested_path: `${parentDir}/${slug}`,
      };
    }
    case "list_left_studies": {
      return [];
    }
    case "resolve_study_location": {
      return { state: "reachable" };
    }
    case "auto_relink_study": {
      return { outcome: "not_found" };
    }
    case "study_readiness": {
      return { kind: "ready" };
    }
    case "check_open_marker": {
      return { status: "clear" };
    }
    case "heartbeat_open_marker": {
      return null;
    }
    case "sync_set_my_coder_name": {
      const next = ((args.coderName as string) || "Ada Lovelace").trim();
      if (
        mockSync.members.some(
          (m) => !m.isYou && m.coderName.toLowerCase() === next.toLowerCase(),
        )
      ) {
        throw new Error(`"${next}" is already in use in this group.`);
      }
      const me = mockSync.members.find((m) => m.isYou);
      const previousName = me?.coderName ?? "";
      if (me) me.coderName = next;
      return { coderName: next, previousName };
    }
    case "sync_now": {
      const pushed = mockSync.pending;
      mockSync.pending = 0;
      mockSync.lastSyncedAt = new Date().toISOString();
      return {
        pushedCoded: pushed,
        pushedCodes: 0,
        pulledCoded: 2,
        pulledCodes: 1,
        pulledInterviews: 1,
        missingTranscripts: [
          // P07 matches the mock's import count (12), P02 does not — one run
          // of the join wizard exercises the verified row and the mismatch
          // override alike.
          { studyLabel: "P07", segmentCount: 12, mismatched: false },
          { studyLabel: "P02", segmentCount: 188, mismatched: true },
        ],
        newCodeNames: ["Tacit gatekeeping"],
        syncedAt: mockSync.lastSyncedAt,
        codedReceipt: { applied: 2, superseded: 0, deferred: 0 },
        codesReceipt: { applied: 1, superseded: 0, deferred: 0 },
        interviewsReceipt: { applied: 1, superseded: 0, deferred: 0 },
        truncated: false,
      };
    }
    case "sync_deep_verify": {
      return {
        localCodedCount: 42,
        localCodeCount: 5,
        remoteCodedCount: 42,
        remoteCodeCount: 5,
        missingRemoteCodedCount: 0,
        missingRemoteCoderNames: [],
        pendingToSendCount: mockSync.pending,
        clockSkewSeconds: 0,
        summaryMessage: "Everything matches. 42 coded passages, 5 codes.",
        rawCodedCursor: "2026-08-22T00:00:00Z",
        rawCodebookCursor: "2026-08-22T00:00:00Z",
        rawInterviewCursor: "2026-08-22T00:00:00Z",
        lastSyncedAt: mockSync.lastSyncedAt,
        needsRepair: false,
        needsSend: mockSync.pending > 0,
        lastError: null,
        lastOutcome: null,
      };
    }
    case "sync_repair": {
      mockSync.lastSyncedAt = new Date().toISOString();
      mockSync.pending = 0;
      return {
        pushedCoded: 0,
        pushedCodes: 0,
        pulledCoded: 2,
        pulledCodes: 1,
        pulledInterviews: 1,
        missingTranscripts: [],
        newCodeNames: [],
        syncedAt: mockSync.lastSyncedAt,
        codedReceipt: { applied: 2, superseded: 0, deferred: 0 },
        codesReceipt: { applied: 1, superseded: 0, deferred: 0 },
        interviewsReceipt: { applied: 1, superseded: 0, deferred: 0 },
        truncated: false,
      };
    }
    case "sync_diagnostics_dump": {
      return (
        "Fleuron Sync Diagnostics\n" +
        "========================\n" +
        "Summary: Everything matches. 42 coded passages, 5 codes.\n" +
        "Local: 42 coded segments, 5 codes\n" +
        "Remote: 42 coded segments, 5 codes\n" +
        "Missing from remote: 0 segments\n" +
        "Pending to send: 0 items\n" +
        "Clock skew: 0s\n" +
        "Last synced: 2026-08-22T12:00:00Z\n" +
        "Cursors: coded=none, codebook=none, interview=none\n"
      );
    }
    case "list_activity":
      return [
        {
          id: "a1",
          coder_name: "Luci Diaz",
          action: "export",
          detail: "2 coded segments",
          created_at: new Date(Date.now() - 36e5 * 30).toISOString(),
        },
        {
          id: "a2",
          coder_name: "Ada Lovelace",
          action: "import_segments",
          detail: "P04-2026-03-11 · 12 segments",
          created_at: new Date(Date.now() - 36e5 * 52).toISOString(),
        },
      ];
    case "read_text_file":
      return "block_id,participant,coder,codes,quote\nq002,P04-2026-03-11,Ada Lovelace,Unwritten rules,\"the people who have been here longest…\"\n";

    case "read_transcript_file":
      // A small valid VTT — twelve turns, matching the count import_segments
      // reports, so the join wizard's link check can verify a match.
      return (
        "WEBVTT\n\n" +
        Array.from(
          { length: 12 },
          (_, i) =>
            `00:${String(i * 2).padStart(2, "0")}:00.000 --> 00:${String(i * 2).padStart(2, "0")}:30.000\nP07: Passage ${i + 1} of the shared transcript.`,
        ).join("\n\n")
      );

    // --- app-level ---------------------------------------------------------
    case "list_recent_projects":
      return [
        {
          path: DEMO.path,
          title: DEMO.title,
          last_opened_at: new Date(Date.now() - 36e5 * 3).toISOString(),
        },
        {
          path: "/Users/demo/Drive/pilot-round.qcproj",
          title: "Pilot round (legacy format)",
          last_opened_at: new Date(Date.now() - 864e5 * 9).toISOString(),
        },
      ];
    case "record_recent_project":
    case "remove_recent_project":
      return [];
    case "get_app_preferences":
      return prefs;
    case "set_app_preferences":
      prefs = args.prefs as typeof prefs;
      return prefs;
    case "get_app_version":
      // No version number on purpose — a hardcoded one here goes stale at every
      // release and this stub has no way to know the real answer.
      return { name: "Fleuron", version: "browser preview", copyright: "Fleuron contributors" };
    case "get_update_status":
      return mockUpdateStatus;
    case "update_check":
      mockUpdateStatus = {
        ...mockUpdateStatus,
        phase: "idle",
        lastCheckedAt: new Date().toISOString(),
        failure: null,
      };
      return mockUpdateStatus;
    case "update_download":
    case "update_cancel_download":
    case "update_install":
      return mockUpdateStatus;
    case "render_report_pdf":
    case "sync_reconcile_pending_unbind":
    case "selftest_sign_in_as":
      return null;
    case "sync_refresh_server_schema":
      return 10;
    case "read_crash_log":
      return "";
    case "generate_diagnostic_report":
      return `=== Fleuron Diagnostic Report ===
Generated: 2026-08-29T12:00:00Z
Notice: This report contains no transcript text, participant labels, or code names.

--- Application ---
Name: Fleuron
Version: 2.2.0
Build commit: dev
Source URL: https://github.com/wilsonhyeh/fleuron
OS: macos
Arch: aarch64

--- Install ---
Executable directory: <home>/Applications/Fleuron.app/Contents/MacOS
Per-user install: true
Poisoned nested directory: clean

--- Library ---
Library directory: <library>
Exists: true
Writable: true
Project folders count: 1

--- Sync ---
Protocol: v2 (Lossless log)
Outbox: 0

--- Storage ---
Project directory: <library>/<name>.fleuron
project.db: present (131072 bytes)
project.db-wal: absent
project.db-shm: absent
Stranded staging restore: clean

--- Updater ---
Status: idle
Staged update residue: clean

--- Crashes ---
Records count: 0 (clean)`;
    case "take_unclean_exit_notice":
      return false;
    case "selftest_online_status":
      return false;
    case "selftest_seed_unbound":
      return "/mock/selftest/unbound";

    default:
      console.warn(`[dev-mock] unhandled command: ${cmd}`);
      return null;
  }
}

export function installDevMock() {
  const w = window as unknown as Record<string, unknown>;
  if (w.__TAURI_INTERNALS__) return;

  let callbackId = 0;
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: () => {},
  };
  w.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
    transformCallback(cb: (payload: unknown) => void, once = false) {
      const id = ++callbackId;
      const key = `_${id}`;
      Object.defineProperty(w, key, {
        value: (result: unknown) => {
          if (once) Reflect.deleteProperty(w, key);
          return cb(result);
        },
        writable: false,
        configurable: true,
      });
      return id;
    },
    async invoke(cmd: string, args: Record<string, unknown> = {}) {
      await new Promise((r) => setTimeout(r, 40));
      try {
        // Fault fixtures intercept BEFORE any handler runs so specs observe
        // the app's real failure branches, not a mocked response shape.
        const fault = activeFault();
        if (fault === "slow" && SLOW_CMDS.has(cmd)) {
          await new Promise((r) => setTimeout(r, SLOW_FIXTURE_DELAY_MS));
        }
        if (fault === "sync-error" && SYNC_FAULT_CMDS.has(cmd)) {
          throw new Error(
            "error sending request for url (https://example-project.supabase.co/rest/v1/): connection refused",
          );
        }
        if (fault === "auth-error" && AUTH_FAULT_CMDS.has(cmd)) {
          throw new Error("Invalid login credentials");
        }
        const result = await handle(cmd, args ?? {});
        recordIpc(cmd, true);
        return result;
      } catch (e) {
        const err = e instanceof Error ? e.message : String(e);
        recordIpc(cmd, false, err);
        throw e;
      }
    },
  };

  document.documentElement.dataset.devMock = "true";
  (w as Record<string, unknown>).__FLEURON_IPC_LOG__ = ipcRing;
  (w as Record<string, unknown>).__FLEURON_CLEAR_IPC_LOG__ = clearIpcRing;
  console.info(
    "%c[Fleuron] browser preview — Tauri IPC is mocked, nothing is written to disk.",
    "color:#8a6410",
  );
}
