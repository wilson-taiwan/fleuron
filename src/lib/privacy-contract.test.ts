/**
 * Static contract test: the trust/privacy disclosure cannot drift from what
 * the code actually does.
 *
 * Three couplings enforced here (all fail loudly if either side changes):
 *   1. Every field Sync Protocol 2 allowlists in src-tauri/src/sync_v2.rs
 *      must be disclosed in src/content/trust-and-permissions.ts, and every
 *      local-only claim there must still be forbidden on the wire.
 *   2. Tauri capabilities must not silently grow; a new permission has to be
 *      added to KNOWN_ALLOWED_PERMISSIONS deliberately or the test fails.
 *   3. No public/in-app text may teach bypassing OS security or claim
 *      compliance certification.
 */
import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const SRC = resolve(import.meta.dirname, "..");
const REPO_ROOT = resolve(SRC, "..");
const SYNC_V2_RS = resolve(REPO_ROOT, "src-tauri/src/sync_v2.rs");
const CAPABILITIES = resolve(REPO_ROOT, "src-tauri/capabilities/default.json");
const TRUST = resolve(SRC, "content/trust-and-permissions.ts");

const SYNC_V2 = () => readFileSync(SYNC_V2_RS, "utf8");
const TRUST_TS = () => readFileSync(TRUST, "utf8");

/** Literals that appear inside object_has_only blocks but are not wire fields. */
const NON_FIELD_LITERALS = new Set([
  "patch", // envelope key carrying patch objects
  "keep_current",
  "accept_proposal",
  "custom", // conflict.resolve enum values
]);

interface OpFields {
  [op: string]: string[];
}

function extractForbidden(): string[] {
  const m = SYNC_V2().match(/let forbidden = \[([\s\S]*?)\];/);
  if (!m) throw new Error("forbidden list not found in sync_v2.rs");
  return [...m[1].matchAll(/"([a-z0-9_]+)"/g)].map((x) => x[1]);
}

function extractAllowlist(): OpFields {
  const src = SYNC_V2();
  const begin = src.indexOf("let valid = match op_kind {");
  if (begin < 0) throw new Error("validate_payload match block not found");
  // Top-level arms are indented exactly eight spaces inside validate_payload.
  const armRe = /\n        "([a-z]+\.[a-z_]+)" =>/g;
  const marks: { op: string; at: number }[] = [];
  for (const m of src.matchAll(armRe)) {
    if (m.index !== undefined && m.index > begin) {
      marks.push({ op: m[1], at: m.index });
    }
  }
  const result: OpFields = {};
  marks.forEach((mark, i) => {
    const end = i + 1 < marks.length ? marks[i + 1].at : src.indexOf("\n    }", mark.at);
    const chunk = src.slice(mark.at, end);
    const fields = [...chunk.matchAll(/"([a-z0-9_]+)"/g)]
      .map((f) => f[1])
      .filter((f) => !NON_FIELD_LITERALS.has(f));
    result[mark.op] = [...new Set(fields)];
  });
  return result;
}

describe("Protocol 2 wire allowlist is fully disclosed", () => {
  it("discloses every allowlisted sync field in the trust content source", () => {
    const allowlist = extractAllowlist();
    const ops = Object.keys(allowlist);
    expect(ops.length).toBeGreaterThanOrEqual(5);
    expect(ops).toEqual(
      expect.arrayContaining(["code.create", "code.patch", "interview.patch", "coding.patch"]),
    );
    const disclosure = TRUST_TS();
    const undisclosed: string[] = [];
    for (const [op, fields] of Object.entries(allowlist)) {
      for (const field of fields) {
        if (!disclosure.includes(`"${field}"`)) undisclosed.push(`${op}:${field}`);
      }
    }
    expect(undisclosed, `fields missing from DATA_BOUNDARY disclosure: ${undisclosed.join(", ")}`).toEqual([]);
  });

  it("maps codebook criteria/examples under the Codebook boundary row tokens", () => {
    const disclosure = TRUST_TS();
    const codebookChunk = disclosure.slice(disclosure.indexOf('"Codebook"'), disclosure.indexOf('"Coding"'));
    for (const f of ["definition", "inclusion_criteria", "exclusion_criteria", "example"]) {
      expect(codebookChunk, `"${f}" must be listed as a synced Codebook token`).toContain(`"${f}"`);
    }
  });

  it("keeps local-only claims out of the wire — forbidden list still forbids them", () => {
    const forbidden = extractForbidden();
    expect(forbidden).toEqual(expect.arrayContaining(["memo", "transcript", "filename"]));
    const allowlisted = new Set(Object.values(extractAllowlist()).flat());
    const leaked = forbidden.filter((f) => allowlisted.has(f));
    expect(leaked, `forbidden fields re-allowlisted: ${leaked.join(", ")}`).toEqual([]);
  });

  it("publishes the canonical publisher-verification notice verbatim", () => {
    const disclosure = TRUST_TS();
    expect(disclosure).toContain(
      "so your operating system cannot verify its publisher automatically",
    );
    expect(disclosure).toContain("Download only from the official release page at https://github.com/wilsonhyeh/fleuron/releases");
    expect(disclosure).toContain("A malware warning, checksum mismatch, or unexpected administrator request means stop.");
  });
});

describe("Tauri capabilities stay minimal and truthful", () => {
  /** Deliberate additions require editing this set AND Trust Center copy. */
  const KNOWN_ALLOWED_PERMISSIONS = [
    "core:default",
    "opener:default",
    "dialog:default",
    "core:window:allow-start-dragging",
    "core:window:allow-minimize",
  ];

  it("grants no capability beyond the known deliberate set", () => {
    const caps = JSON.parse(readFileSync(CAPABILITIES, "utf8")) as {
      permissions: string[];
    };
    const extra = caps.permissions.filter((p) => !KNOWN_ALLOWED_PERMISSIONS.includes(p));
    expect(extra, `unexpected new permissions: ${extra.join(", ")}`).toEqual([]);
  });

  it("capability config never names a hardware/sensor permission", () => {
    const capsText = readFileSync(CAPABILITIES, "utf8").toLowerCase();
    for (const sensor of ["camera", "microphone", "screen-recording", "accessibility", "geolocation", "bluetooth", "notifications"]) {
      expect(capsText).not.toContain(sensor);
    }
  });
});

describe("prohibited guidance never appears in user-facing text", () => {
  /**
   * Fails if any scanned file teaches defeating an OS security control,
   * claims compliance certification, uses "opaque IDs" framing, or suggests
   * adding firewall exceptions. Negations ("do not disable…") are fine.
   */
  function* markdownAndContentFiles(): Generator<{ file: string; text: string }> {
    const readme = resolve(REPO_ROOT, "README.md");
    yield { file: "README.md", text: readFileSync(readme, "utf8") };
    try {
      for (const name of readdirSync(resolve(REPO_ROOT, "docs"))) {
        if (!name.endsWith(".md")) continue;
        yield {
          file: `docs/${name}`,
          text: readFileSync(resolve(REPO_ROOT, "docs", name), "utf8"),
        };
      }
    } catch {
      /* docs dir contents grow during v1.2; absence is not a violation */
    }
    for (const name of readdirSync(resolve(SRC, "content"))) {
      if (!name.endsWith(".ts")) continue;
      yield {
        file: `src/content/${name}`,
        text: readFileSync(resolve(SRC, "content", name), "utf8"),
      };
    }
  }

  function sentences(text: string): string[] {
    return text.split(/[.!?\n]/).map((s) => s.trim()).filter(Boolean);
  }

  function violationIn(text: string): string | null {
    const CONTROL =
      /(Gatekeeper|Defender|SmartScreen|Smart App Control|Controlled Folder Access|User Account Control|\bUAC\b)/gi;
    const BYPASS_VERB =
      /\b(disable|turn\s+off|bypass|weaken|defeat)\s+(?:\w+\s+){0,2}?(Gatekeeper|Defender|SmartScreen|Smart App Control|Controlled Folder Access|User Account Control|\bUAC\b)/i;
    const stripQuarantine = /\b(strip|remove)\s+(?:\w+\s+){0,3}?quarantine/i;
    const bypassVerb = new RegExp(BYPASS_VERB.source, "gi");
    for (const s of sentences(text)) {
      // A bypass verb directly governing a control name is banned UNLESS a
      // negation ("do not disable…") governs the same clause.
      let m: RegExpExecArray | null;
      while ((m = bypassVerb.exec(s)) !== null) {
        const before = s.slice(Math.max(0, m.index - 80), m.index);
        if (!/\b(do not|don't|never|without)\b[^.]*(disable|turn off)?$/i.test(before)) {
          return `security-control bypass: "${s.slice(0, 120)}"`;
        }
      }
      if (
        !/\b(do not|don't|never|no)\b/i.test(s) &&
        (stripQuarantine.test(s) || (/\bbypass\b/i.test(s) && CONTROL.test(s)))
      )
        return `security-control bypass: "${s.slice(0, 120)}"`;
      if (/HIPAA|IRB/i.test(s)) return `compliance claim: "${s.slice(0, 120)}"`;
      if (/opaque\s+ids?/i.test(s)) return `'opaque IDs' framing: "${s.slice(0, 120)}"`;
      if (/\bxattr\b/.test(s)) return `xattr guidance: "${s.slice(0, 120)}"`;
      if (/Properties\s*(→|->)\s*Unblock/i.test(s)) return `Unblock guidance: "${s.slice(0, 120)}"`;
      if (/right-?click\s*(→|->)?\s*open/i.test(s)) return `right-click Open guidance: "${s.slice(0, 120)}"`;
      if (/\b(add|create|allow)\b[^.]{0,60}\bfirewall\b[^.]{0,30}\b(exception|rule)\b/i.test(s) && !/\bno\s+(firewall|inbound)/i.test(s))
        return `firewall exception advice: "${s.slice(0, 120)}"`;
      if (/(switch|turning|turn)\s+(out\s+of|off)\s+(S mode)?/i.test(s) && /S mode/i.test(s) && !/not recommend|unsupported|do not|don't|stop/i.test(s))
        return `S-mode switch advice: "${s.slice(0, 120)}"`;
    }
    return null;
  }

  it("README, docs, and in-app content contain no prohibited guidance", () => {
    const violations: string[] = [];
    for (const { file, text } of markdownAndContentFiles()) {
      const v = violationIn(text);
      if (v) violations.push(`${file}: ${v}`);
    }
    expect(violations, violations.join("\n")).toEqual([]);
  });

  it("source-available does not equal safe anywhere in our own copy", () => {
    for (const { file, text } of markdownAndContentFiles()) {
      expect(/because it('s| is) open[- ]source[, ]*(it )?(is )?safe/i.test(text), `${file} claims open-source proves safe`).toBe(false);
    }
  });
});
