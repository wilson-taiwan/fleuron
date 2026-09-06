/**
 * The single typed source for every public trust, warning, privacy, and
 * permission claim Fleuron makes.
 *
 * Why one module: v1.1 drifted — README, the external guide, the in-app
 * guide, About, and setup copy disagreed about sync fields and made claims
 * that were false (compliance certification, unknowable-to-the-server sync,
 * Keychain on macOS). Every surface below renders from these exports; when
 * wording must change, it changes here or a contract test fails.
 *
 * Three rules bind everything in this file:
 *   1. Never call an OS warning a "false positive" / "harmless" / "just
 *      cautious", and never teach bypassing a security control.
 *   2. Disclose exact synced fields; never imply that sync traffic is
 *      unknowable to the service operator, and never claim compliance
 *      certification.
 *   3. State plainly what signing/provenance does NOT provide (no Apple
 *      Developer ID/notarization, no Windows Authenticode publisher).
 */

import { isMac } from "../lib/platform";

// ---------------------------------------------------------------------------
// Official URLs
// ---------------------------------------------------------------------------

const REPO = "https://github.com/wilson-taiwan/fleuron";

export const OFFICIAL_URLS = {
  website: "https://fleuron.study/",
  installGuideWeb: "https://fleuron.study/install/",
  privacyWeb: "https://fleuron.study/privacy/",
  repository: REPO,
  releases: `${REPO}/releases`,
  latestRelease: `${REPO}/releases/latest`,
  installGuide: `${REPO}/blob/main/docs/INSTALLING.md`,
  privacyGuide: `${REPO}/blob/main/docs/PRIVACY-AND-PERMISSIONS.md`,
  itDeployment: `${REPO}/blob/main/docs/IT-DEPLOYMENT.md`,
  support: `${REPO}/blob/main/SUPPORT.md`,
  security: `${REPO}/blob/main/SECURITY.md`,
  issues: `${REPO}/issues`,
} as const;

/** The two manual downloads. Other release assets are updater internals or optional QA runners. */
export const CANONICAL_ASSETS = {
  macos: "Fleuron_2.7.0_universal.dmg",
  windows: "Fleuron_2.7.0_x64-setup.exe",
} as const;

/**
 * Assets that ship in every release but are NOT alternate downloads:
 * updater archive + signatures + manifest.
 */
export const UPDATER_INFRASTRUCTURE_ASSETS = [
  "Fleuron_universal.app.tar.gz",
  "Fleuron_universal.app.tar.gz.sig",
  "Fleuron_2.7.0_x64-setup.exe.sig",
  "latest.json",
] as const;

// ---------------------------------------------------------------------------
// Canonical publisher-verification paragraph
// ---------------------------------------------------------------------------

/**
 * Used verbatim by docs/INSTALLING.md, the Trust & permissions center, and
 * the GitHub Release body. Keep the line breaks out of this constant; prose
 * wraps itself at render time.
 */
export const PUBLISHER_VERIFICATION_NOTICE =
  "Fleuron is an independent open-source application. This build does not yet carry an Apple Developer ID/notarization or Windows Authenticode publisher signature, so your operating system cannot verify its publisher automatically. Download only from the official release page at https://github.com/wilson-taiwan/fleuron/releases. Continue only when the version, filename, and warning match this guide. A malware warning, checksum mismatch, or unexpected administrator request means stop.";

/**
 * What each provenance signal actually proves — and what it does not. None of
 * them gives the OS a paid publisher identity or a malware-notarization result.
 */
export const PROVENANCE_SIGNALS = [
  {
    signal: "GitHub artifact attestation",
    proves:
      "The downloaded file was built by this repository's workflow from a specific public commit.",
    doesNotProve:
      "It is not an Apple or Microsoft publisher identity and says nothing about malware scanning.",
  },
  {
    signal: "SHA-256 checksum match",
    proves: "Your file is byte-for-byte identical to the published asset.",
    doesNotProve: "Anything about who published it or whether its code is safe.",
  },
  {
    signal: "Tauri/minisign update signature",
    proves: "An installed app only accepts updates produced by the same project key.",
    doesNotProve: "Anything about the first download — it protects updates, not initial install.",
  },
] as const;

// ---------------------------------------------------------------------------
// Platform support matrix
// ---------------------------------------------------------------------------

export type SupportTier = "supported" | "best-effort" | "unsupported";

export interface SupportRow {
  platform: string;
  tier: SupportTier;
  note: string;
}

export const SUPPORT_MATRIX: SupportRow[] = [
  {
    platform: "macOS 14 (Sonoma) or newer — Intel and Apple Silicon",
    tier: "supported",
    note: "Officially documented and manually validated. One universal DMG covers both chips.",
  },
  {
    platform: "macOS 10.15–13",
    tier: "best-effort",
    note: "Technically supported by the build but not validated per release.",
  },
  {
    platform: "Windows 11 x64 on a personal machine",
    tier: "supported",
    note: "Standard user account; installs per-user with no administrator prompt.",
  },
  {
    platform: "Windows 10 (out of support since 2025-10-14), Windows on ARM",
    tier: "best-effort",
    note: "No x64 ARM build ships; Intel emulation is untested territory.",
  },
  {
    platform: "Windows S mode, managed-policy machines (WDAC/AppLocker)",
    tier: "unsupported",
    note: "S mode permits Store apps only. Managed devices are controlled by your IT department.",
  },
];

// ---------------------------------------------------------------------------
// OS warning response contract
// ---------------------------------------------------------------------------

export type WarningMeaning =
  | "expected"
  | "stop"
  | "release-blocking";

export interface WarningCard {
  /** Short label shown in UI, e.g. "Apple cannot verify the developer". */
  signal: string;
  meaning: WarningMeaning;
  /** What this warning actually means for Fleuron. Never "harmless". */
  explanation: string;
  /** What we tell the user to do. Empty string for stop conditions where the action is simply "stop". */
  userAction: string;
}

export const MACOS_WARNING_CARDS: WarningCard[] = [
  {
    signal: "\u201cApple cannot check the app for malicious software\u201d / \u201ccannot verify the developer\u201d",
    meaning: "expected",
    explanation:
      "Expected for the official non-notarized, ad-hoc signed build. macOS is saying it cannot verify the publisher — not that it found malware.",
    userAction:
      "Confirm you downloaded from github.com/wilson-taiwan/fleuron/releases, then System Settings \u2192 Privacy & Security \u2192 Open Anyway, authenticate, and Open. No Terminal steps are needed.",
  },
  {
    signal: "\u201cWill damage your computer\u201d / malware alert / \u201cdamaged and cannot be opened\u201d",
    meaning: "stop",
    explanation:
      "A strong warning like malware or damaged means corruption or a genuine problem. It is not the ordinary unsigned-publisher warning.",
    userAction:
      "Stop. Do not override it. Delete the download, compare the file against the official SHA-256 digest, and if an official re-download repeats the error, file an install issue.",
  },
  {
    signal: "Files & Folders / network / removable volume access prompt after you picked that location",
    meaning: "expected",
    explanation:
      "macOS asks when an app touches Documents, Desktop, Downloads, network volumes, or removable drives because you selected one of those locations.",
    userAction:
      "Allow, or cancel and choose a different folder inside Fleuron's picker. Denial is fine — Fleuron offers inline recovery and never requests Full Disk Access.",
  },
];

export const WINDOWS_WARNING_CARDS: WarningCard[] = [
  {
    signal: "SmartScreen: \u201cWindows protected your PC\u201d / unrecognized app, Unknown publisher",
    meaning: "expected",
    explanation:
      "Reputation warning for a newly published, non-Authenticode-signed installer. It is the expected result of an unsigned-but-official download.",
    userAction:
      "Confirm the download URL, version, and filename, then More info \u2192 Run anyway.",
  },
  {
    signal: "Microsoft Defender reports a threat / quarantines the installer",
    meaning: "stop",
    explanation:
      "A Defender detection is not the reputation warning. Treat it as real until investigated.",
    userAction:
      "Stop. Do not restore or allow the file. Preserve the exact warning text and file an issue from a safe device.",
  },
  {
    signal: "User Account Control asks for administrator credentials to install",
    meaning: "stop",
    explanation:
      "Unexpected: Fleuron installs per-user with no elevation. An admin prompt means something is off with the installer or your machine's policy.",
    userAction:
      "Cancel. Do not enter credentials. File an install issue.",
  },
  {
    signal: "Smart App Control blocks the app",
    meaning: "stop",
    explanation:
      "Smart App Control has no per-app override; unblocking would mean weakening it.",
    userAction:
      "Stop. Do not disable Smart App Control to run Fleuron.",
  },
  {
    signal: "Windows S mode blocks installation",
    meaning: "stop",
    explanation:
      "S mode only runs Store apps; Fleuron is not distributed through the Store.",
    userAction:
      "Stop. Fleuron is unsupported in S mode. Do not switch out of S mode for this.",
  },
  {
    signal: "WDAC / App Control / AppLocker / managed SmartScreen blocks the app",
    meaning: "stop",
    explanation: "Your organization's device policy decides which apps may run.",
    userAction:
      "Contact your IT department and share docs/IT-DEPLOYMENT.md. Never bypass organizational policy.",
  },
  {
    signal: "Controlled Folder Access denied a write to a protected folder",
    meaning: "expected",
    explanation:
      "Ransomware protection blocked a write into a folder you had protected. Nothing malicious happened.",
    userAction:
      "Choose another folder (the default %USERPROFILE%\\Fleuron avoids protected folders). After verifying the source, you or IT may allow-list Fleuron under your own policy.",
  },
  {
    signal: "Windows Settings \u2192 Privacy \u2192 File system does not list Fleuron",
    meaning: "expected",
    explanation:
      "Desktop applications are governed differently from Store apps there. Absence from that panel is normal.",
    userAction: "Nothing — do not look for Fleuron in that panel.",
  },
  {
    signal: "WebView2 missing or damaged",
    meaning: "expected",
    explanation:
      "Rare: Windows 11 normally includes the Evergreen WebView2 Runtime. Fleuron never downloads, bundles, or repairs it silently.",
    userAction:
      "Use Microsoft's official WebView2 repair/install page, or contact IT on managed devices.",
  },
  {
    signal: "An inbound firewall prompt appears for Fleuron",
    meaning: "stop",
    explanation:
      "Fleuron only initiates outbound HTTPS/WSS on port 443. It never listens for inbound connections, so an inbound prompt should not happen.",
    userAction: "Deny it and report it via an install issue.",
  },
];

export function warningCardsForCurrentPlatform(): WarningCard[] {
  return isMac ? MACOS_WARNING_CARDS : WINDOWS_WARNING_CARDS;
}

// ---------------------------------------------------------------------------
// Files & permissions
// ---------------------------------------------------------------------------

export const FILE_ACCESS_NOTES = [
  {
    title: "Where projects live by default",
    detail:
      "New studies are created in a local working library: ~/Fleuron on macOS, %USERPROFILE%\\Fleuron on Windows. These locations avoid Documents/Desktop, which both operating systems protect and which cloud sync (OneDrive/iCloud) can redirect. Existing projects are never moved; their recorded paths stay in Recents until you remove them.",
  },
  {
    title: "Choosing files or folders yourself is the consent",
    detail:
      "Fleuron uses your operating system's native open/save pickers. When you pick a transcript, study folder, or export destination, that choice IS the permission grant — Fleuron shows no extra pre-prompt before the picker and asks nothing beyond it.",
  },
  {
    title: "Folder scans read the folder you chose, only immediately inside it",
    detail:
      "When linking transcripts from a folder, Fleuron reads supported files (.vtt, .srt, .txt, .md, .csv, .tsv, .docx) directly inside that folder — not subfolders — to match participant labels and content hashes. Processing happens on this computer; transcript text is not uploaded.",
  },
  {
    title: "If access is denied",
    detail:
      "Offline volumes, revoked permissions (macOS Files & Folders, Windows Controlled Folder Access), and full disks surface as readable explanations inside Fleuron, with actions like Locate folder, Choose another folder, or Remove from recent. Raw operating-system error strings stay out of the main interface.",
  },
] as const;

// ---------------------------------------------------------------------------
// Local vs collaboration data boundary
// ---------------------------------------------------------------------------

/**
 * `syncedFields` tokens exist so the contract test can statically cross-check
 * this disclosure against the Protocol 2 wire allowlist in
 * src-tauri/src/sync_v2.rs. If Rust allows a new field, this table must name
 * it or the test fails; if this table names a local-only field and Rust starts
 * allowlisting it, the test also fails.
 */
export interface DataBoundaryRow {
  category: string;
  syncedWhenCollaborating: string;
  keptLocal: string;
  /** Field names appearing in the Sync Protocol 2 allowlist that this row discloses. */
  syncFieldTokens?: string[];
}

export const DATA_BOUNDARY_SUMMARY =
  "Collaboration sync excludes transcript text and memo fields. It shares account, study, codebook, and coding metadata over encrypted HTTPS/WSS. Quotes copied into codebook fields sync as written; exports may leave your device through sharing or a synced folder.";

export const DATA_BOUNDARY: DataBoundaryRow[] = [
  {
    category: "Account and access",
    syncedWhenCollaborating:
      "Account email and Supabase authentication metadata; entitlement state; study membership, role, coder display name, device/readiness and sync metadata.",
    keptLocal: "The refresh token file itself; local app preferences and paths. Authentication sends credential values to the configured sign-in service.",
  },
  {
    category: "Study / interview identity",
    syncedWhenCollaborating:
      "Shared study identity/title; de-identified study/participant label; interview ID; segment count; deterministic content hash; revisions/timestamps/deletion state.",
    keptLocal:
      "Transcript raw files and text; filenames/paths; interview date; interviewer names; diagnosis fields.",
    syncFieldTokens: ["study_label", "segment_count", "content_hash", "deleted"],
  },
  {
    category: "Codebook",
    syncedWhenCollaborating:
      "Code IDs; name; definition; inclusion criteria; exclusion criteria; examples; parent/hierarchy; color; sort order; retired/deleted state; versions/timestamps.",
    keptLocal:
      "Nothing within these text fields is automatically de-identified — researchers must author codes, definitions, criteria, and examples safely.",
    syncFieldTokens: [
      "name",
      "definition",
      "inclusion_criteria",
      "exclusion_criteria",
      "example",
      "parent_id",
      "color",
      "sort_order",
      "is_retired",
    ],
  },
  {
    category: "Coding",
    syncedWhenCollaborating:
      "Coder attribution; interview/segment/code IDs; character start/end offsets; add/remove/deletion state; conflict/version/timestamp metadata.",
    keptLocal: "Verbatim quote text; coding memos and analytic memos.",
    syncFieldTokens: [
      "interview_id",
      "segment_id",
      "code_id",
      "char_start",
      "char_end",
      "adds",
      "removes",
      "conflict_id",
      "resolution",
      "value",
    ],
  },
  {
    category: "Output and diagnostics",
    syncedWhenCollaborating: "Nothing, automatically.",
    keptLocal:
      "Exports, project databases/backups, crash logs, device-local activity. These may leave your device when shared or saved in a cloud-synced folder.",
  },
];

/** Fields that must NEVER appear in the sync allowlist — checked by test. */
export const ALWAYS_LOCAL_CLAIMS = [
  { label: "Transcript text / verbatim quotes", token: "transcript_text" },
  { label: "Filenames and paths", token: "filename" },
  { label: "Interviewer names", token: "interviewer" },
  { label: "Diagnosis fields", token: "diagnosis" },
  { label: "Memos (coding + analytic)", token: "memo" },
] as const;

// ---------------------------------------------------------------------------
// Network behavior
// ---------------------------------------------------------------------------

export interface NetworkBehavior {
  purpose: string;
  endpoint: string;
  protocol: string;
  enabledByDefault: boolean;
}

export const NETWORK_BEHAVIOR: NetworkBehavior[] = [
  {
    purpose: "Update checks at startup and every four hours (is a newer release published?)",
    endpoint: "api.github.com — wilson-taiwan/fleuron releases",
    protocol: "Outbound HTTPS",
    enabledByDefault: true,
  },
  {
    purpose: "Downloading and installing an update after you approve it",
    endpoint: "github.com/wilson-taiwan/fleuron/releases",
    protocol: "Outbound HTTPS",
    enabledByDefault: false,
  },
  {
    purpose: "Sign-in, session renewal and collaboration (study/codebook/coding metadata)",
    endpoint: "The configured Supabase service for your study",
    protocol: "Outbound HTTPS / WSS (encrypted transport)",
    enabledByDefault: false,
  },
];

export const NETWORK_NOTES = [
  "Working locally makes no Supabase connection — only the quiet GitHub update check runs (and you can turn that off in Settings).",
  "Fleuron opens no inbound port and needs no firewall exception. Outbound-only traffic is how desktop apps normally behave.",
  "HTTPS/WSS encrypts data in transit. That protects it on the wire; it does not make the contents unknowable to the service operator hosting collaboration sync.",
] as const;

// ---------------------------------------------------------------------------
// Stored sign-in
// ---------------------------------------------------------------------------

export const STORED_SIGN_IN = {
  macos:
    "Only your refresh token is stored, in a session.json file inside the app data folder with permissions set so only your user account can read it (mode 0600). macOS Keychain is deliberately not used while builds are ad-hoc signed — each rebuild changes the code signature, and Keychain would then demand approval again on every launch. Signing out deletes the stored token. Your password is never stored anywhere.",
  windows:
    "Only your refresh token is stored, encrypted with Windows DPAPI bound to your current Windows user account on this machine (session.dpapi). It cannot be decrypted by another user account or another computer, and DPAPI never prompts. Signing out deletes every stored session artifact. Your password is never stored anywhere.",
  universalNote:
    "Sign-in exists only for optional collaboration. Local coding needs no account. Stored credential files stay on this computer; credential values are sent to the configured sign-in service for authentication and session renewal. Your password is never stored anywhere.",
} as const;

// ---------------------------------------------------------------------------
// Capabilities Fleuron does NOT request
// ---------------------------------------------------------------------------

export const NOT_REQUESTED_CAPABILITIES = [
  "Camera",
  "Microphone",
  "Screen recording",
  "Accessibility",
  "Location",
  "Contacts",
  "Calendar",
  "Notifications",
  "Bluetooth",
  "Local-network discovery",
  "Full Disk Access",
  "Inbound network listener / firewall exception",
] as const;

// ---------------------------------------------------------------------------
// Build provenance surfaced in Trust Center / About
// ---------------------------------------------------------------------------

export interface BuildDetails {
  version: string;
  buildCommit: string;
  platform: string;
}

// ---------------------------------------------------------------------------
// Support entry points
// ---------------------------------------------------------------------------

export const SUPPORT_CHANNELS = [
  {
    label: "Install help or bugs",
    where: "GitHub Issues (with guided forms)",
    url: OFFICIAL_URLS.issues,
    caution:
      "Do not attach transcripts, quotes, participant/study identifiers, project databases, tokens, or unredacted logs/screenshots.",
  },
  {
    label: "Security reports",
    where: "GitHub private vulnerability reporting",
    url: `${OFFICIAL_URLS.security}`,
    caution:
      "Report confidentially through GitHub's \u201cReport a vulnerability\u201d flow — not a public issue.",
  },
  {
    label: "Deploying in an organization",
    where: "IT deployment guide",
    url: OFFICIAL_URLS.itDeployment,
    caution: "",
  },
] as const;

export const CRASH_LOG_CAUTION =
  "Crash logs stay on this machine unless you copy them somewhere. They may contain file paths or diagnostic messages — review and redact before sharing.";
