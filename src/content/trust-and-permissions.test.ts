/**
 * Tests for the central trust content contract itself: the canonical notice
 * stays verbatim-capable, the data-boundary table covers every sensitive
 * category including codebook criteria/examples, warning cards keep the
 * expected-vs-stop distinction, and no banned framing appears.
 */
import { describe, expect, it } from "vitest";
import {
  ALWAYS_LOCAL_CLAIMS,
  CANONICAL_ASSETS,
  DATA_BOUNDARY,
  MACOS_WARNING_CARDS,
  NOT_REQUESTED_CAPABILITIES,
  NETWORK_BEHAVIOR,
  OFFICIAL_URLS,
  PROVENANCE_SIGNALS,
  PUBLISHER_VERIFICATION_NOTICE,
  STORED_SIGN_IN,
  SUPPORT_MATRIX,
  WINDOWS_WARNING_CARDS,
} from "./trust-and-permissions";

describe("publisher verification notice", () => {
  it("matches the canonical wording exactly", () => {
    expect(PUBLISHER_VERIFICATION_NOTICE).toBe(
      "Fleuron is an independent open-source application. This build does not yet carry an Apple Developer ID/notarization or Windows Authenticode publisher signature, so your operating system cannot verify its publisher automatically. Download only from the official release page at https://github.com/wilson-taiwan/fleuron/releases. Continue only when the version, filename, and warning match this guide. A malware warning, checksum mismatch, or unexpected administrator request means stop.",
    );
  });

  it("names all three provenance signals and what they do not prove", () => {
    const joined = PROVENANCE_SIGNALS.map((s) => `${s.signal}: ${s.proves}`).join("\n");
    expect(joined).toContain("attestation");
    expect(joined).toContain("SHA-256");
    expect(joined).toContain("minisign");
    for (const s of PROVENANCE_SIGNALS) {
      expect(s.doesNotProve.length).toBeGreaterThan(10);
    }
  });
});

describe("data boundary", () => {
  it("covers all five disclosed categories", () => {
    const categories = DATA_BOUNDARY.map((r) => r.category);
    expect(categories).toEqual([
      "Account and access",
      "Study / interview identity",
      "Codebook",
      "Coding",
      "Output and diagnostics",
    ]);
  });

  it("states codebook text fields are synced and not auto-de-identified", () => {
    const codebook = DATA_BOUNDARY.find((r) => r.category === "Codebook");
    expect(codebook?.syncedWhenCollaborating).toContain("inclusion criteria");
    expect(codebook?.syncedWhenCollaborating).toContain("examples");
    expect(codebook?.keptLocal.toLowerCase()).toContain("de-identified");
  });

  it("keeps transcript text, memos, filenames, interviewer names local", () => {
    const allLocal = DATA_BOUNDARY.map((r) => r.keptLocal).join(" ");
    for (const claim of ALWAYS_LOCAL_CLAIMS) {
      expect(allLocal, `${claim.label} must appear as kept local`).toBeTruthy();
    }
    const joined = [
      DATA_BOUNDARY.find((r) => r.category === "Coding")?.keptLocal ?? "",
      DATA_BOUNDARY.find((r) => r.category === "Study / interview identity")?.keptLocal ?? "",
    ].join(" ");
    expect(joined).toContain("Verbatim quote text");
    expect(joined).toContain("memos");
    expect(joined).toContain("filenames/paths");
    expect(joined).toContain("interviewer names");
    expect(joined).toContain("diagnosis fields");
  });
});

describe("warning response contract", () => {
  it("macOS separates expected ordinary warnings from stop conditions", () => {
    const signals = MACOS_WARNING_CARDS.map((c) => c.meaning);
    expect(signals).toContain("expected");
    expect(signals).toContain("stop");
    const damaged = MACOS_WARNING_CARDS.find((c) => c.signal.includes("damage"));
    expect(damaged?.meaning).toBe("stop");
    const openAnyway = MACOS_WARNING_CARDS.find((c) => c.userAction.includes("Open Anyway"));
    expect(openAnyway?.meaning).toBe("expected");
  });

  it("Windows never gives bypass instructions for hard-stop conditions", () => {
    const smartAppControl = WINDOWS_WARNING_CARDS.find((c) => c.signal.includes("Smart App Control"));
    expect(smartAppControl?.meaning).toBe("stop");
    expect(smartAppControl?.userAction.toLowerCase()).toContain("do not disable");
    const smode = WINDOWS_WARNING_CARDS.find((c) => c.signal.includes("S mode"));
    expect(smode?.meaning).toBe("stop");
    const defender = WINDOWS_WARNING_CARDS.find((c) => c.signal.includes("Defender"));
    expect(defender?.meaning).toBe("stop");
    const uac = WINDOWS_WARNING_CARDS.find((c) => c.signal.includes("administrator credentials"));
    expect(uac?.meaning).toBe("stop");
    const smartscreen = WINDOWS_WARNING_CARDS.find((c) => c.signal.includes("SmartScreen"));
    expect(smartscreen?.meaning).toBe("expected");
    expect(smartscreen?.userAction).toContain("Run anyway");
  });

  it("never softens strong warnings anywhere in card copy", () => {
    for (const card of [...MACOS_WARNING_CARDS, ...WINDOWS_WARNING_CARDS]) {
      expect(card.explanation.toLowerCase()).not.toMatch(/harmless|false positive|just apple|just microsoft|being cautious/);
    }
  });
});

describe("support matrix and capabilities", () => {
  it("documents macOS universal minimum 10.15 with supported tier on 14+", () => {
    const mac = SUPPORT_MATRIX.filter((r) => r.platform.startsWith("macOS"));
    expect(mac.some((r) => r.tier === "supported" && r.platform.includes("14"))).toBe(true);
    expect(mac.some((r) => r.note.includes("universal"))).toBe(true);
    expect(mac.some((r) => r.platform.includes("10.15"))).toBe(true);
  });

  it("marks S mode unsupported without a switch recommendation", () => {
    const sMode = SUPPORT_MATRIX.find((r) => r.platform.includes("S mode"));
    expect(sMode?.tier).toBe("unsupported");
  });

  it("lists every capability Fleuron does not request", () => {
    for (const cap of ["Camera", "Microphone", "Screen recording", "Accessibility", "Location", "Full Disk Access"]) {
      expect(NOT_REQUESTED_CAPABILITIES).toContain(cap);
    }
  });
});

describe("network, sign-in storage, and canonical assets", () => {
  it("describes update checks as default-on outbound-only HTTPS", () => {
    const updates = NETWORK_BEHAVIOR[0];
    expect(updates.enabledByDefault).toBe(true);
    expect(updates.protocol).toMatch(/Outbound HTTPS/);
  });

  it("collaboration is opt-in; transport encryption not conflated with secrecy", () => {
    const collab = NETWORK_BEHAVIOR[2];
    expect(collab.enabledByDefault).toBe(false);
    expect(collab.protocol).toContain("WSS");
  });

  it("macOS storage explains 0600 file and why Keychain is unused; Windows DPAPI", () => {
    expect(STORED_SIGN_IN.macos).toContain("0600");
    expect(STORED_SIGN_IN.macos.toLowerCase()).toContain("keychain");
    expect(STORED_SIGN_IN.windows).toContain("DPAPI");
    expect(STORED_SIGN_IN.windows.toLowerCase()).not.toContain("credential manager");
    expect(STORED_SIGN_IN.universalNote).toContain("never stored");
  });

  it("canonical assets are exactly the two manual downloads", () => {
    expect(CANONICAL_ASSETS.macos).toBe("Fleuron_2.6.0_universal.dmg");
    expect(CANONICAL_ASSETS.windows).toBe("Fleuron_2.6.0_x64-setup.exe");
    expect(OFFICIAL_URLS.website).toBe("https://fleuron.study/");
    expect(OFFICIAL_URLS.installGuideWeb).toBe("https://fleuron.study/install/");
    expect(OFFICIAL_URLS.privacyWeb).toBe("https://fleuron.study/privacy/");
    expect(OFFICIAL_URLS.releases).toContain("/releases");
  });
});
