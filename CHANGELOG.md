# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.7.0]

### Fixed

- **macOS window reopen from Dock:** Clicking the Fleuron dock icon when all windows are closed on macOS now reopens the application window instead of remaining inactive.
- **Accidental deletion prevention:** Permanent file destruction via direct unlinking is eliminated; deleting a study moves its files safely to the system Trash or Recycle Bin with clear recovery steps.
- **Collaborative leave safety:** Leaving a shared study preserves local data and remote project integrity; sole members cannot orphan shared studies, and non-admin participants are protected from accidental project destruction.

### Added

- **Guided setup flow:** A four-step guided setup (Study Details, Collaborators, Initial Files, Summary) replaces modal setup prompts, with non-blocking "Finish setup later" and preserved progress stored directly in the study file.
- **Whole-interview speaker management:** A dedicated "Manage Speakers" modal in the transcript menu allows viewing turn and coded segment counts per speaker, renaming all instances across the interview, and merging speakers with rollback support.
- **Study lifecycle coordinator and preflight:** Intelligent detection and user guidance for missing files, moved directories, locked stores, and offline volumes before opening or creating studies.
- **Preserved local backups:** "Save local copy" exports a verified SQLite snapshot with external transcript references outside the project folder for portable offline archiving.

---

## [2.6.0]

### Fixed

- **Truthful autosave and revision tracking:** Autosave now verifies target and revision acknowledgements before reporting saved, preventing false "Saved" confirmations on write failures and ensuring delayed saves never clear dirty state when newer edits are present.
- **Passage-note draft retention:** Notes no longer discard uncommitted drafts on collapse, navigation, interview switching, or filter changes; returning or reopening restores active drafts without loss.
- **Unified inline note editing:** Removed the competing side-rail note editor; all passage note creation and editing now converge on a single inline note editor docked beneath the quoted passage.
- **Consolidated alert host:** Removed duplicate toast stack from the workspace layout, ensuring notifications render from a single region without overlapping banners.
- **Code pill contrast and export copy:** Solid code labels use dynamic luminance contrast across both themes, and export dialog options replace technical format descriptions with clear, plain-language labels.

### Added

- **Private local draft crash recovery:** Unsaved passage notes and interview memos are backed up continuously to a private local SQLite recovery database (`note-recovery.sqlite3`), completely separated from project files, backups, exports, and sync payloads.
- **Conflict comparison and orphan draft management:** When committed notes change beneath an active draft, a side-by-side comparison modal lets you choose which version to keep; drafts whose source coding was deleted are preserved in a local recovery drawer with Copy and Discard options.
- **Guarded project departure and native exit:** Closing a project, quitting the app (including macOS Cmd+Q and Windows Alt+F4), and updater installation are intercepted with a Save all / Discard changes / Cancel dialog whenever unfinished drafts exist.
- **Three-way export preflight:** Exporting with dirty drafts prompts with "Save all and export", "Export saved notes only", or "Cancel", ensuring all generated exports derive purely from committed study data.
- **Streamlined transcript navigation:** Consolidated interview switching into a single selector in the reading header, with explicit "Back" and "Forward" history labels.

---

## [2.5.0]

### Fixed

- **Sync Protocol 2 whole-turn coding repair:** Whole-turn coding edges now omit character range keys on new clients and permit explicit null/null ranges on the server, draining queued 2.4.x operations without data loss or outbox resets.
- **Realtime subscription restoration:** Restored table-level `SELECT` privilege on `sync_project_heads` for authenticated users, enabling live Realtime change hints while preserving member-only row-level security.
- **Scoped SQLSTATE 22023 diagnostics:** Sanitized server error messages from `sync_v2_apply` are now exposed in the sync diagnostics panel while keeping internal database messages from other RPCs generic.
- **Solid-pill text contrast repair:** Dark-mode code pills with solid backgrounds now dynamically calculate high-contrast text color based on background luminance, ensuring WCAG AA legibility across all custom code colors.
- **App launch and window focus:** On macOS, launching or relaunching Fleuron (including after an updater restart) properly brings the main window to the foreground.

### Added

- **Inline speaker reassignment with atomic undo:** Click any speaker label in a transcript to change the speaker for that turn or all subsequent turns from that speaker, with an 8-second toast offering one-click atomic undo.
- **Local transcript reviewed marks and resume:** Mark transcript segments as reviewed with a check badge; returning to an interview automatically scrolls to the first unreviewed segment so you can pick up right where you left off.
- **Transcript toolbar filtering and layout polish:** Combined code and speaker filtering into a single toolbar popover with Any/All matching logic, active filter chips, and Alt-click code isolation; simplified the transcript header and centered the collapsible codebook rail toggle on the divider.
- **Back and forward navigation:** Navigate across visited interviews with dedicated toolbar buttons and `Cmd/Ctrl+[` / `Cmd/Ctrl+]` shortcuts, preserving exact scroll and selection state.
- **Targeted copy and redaction clarity:** Added contextual InfoTips across settings, coach, and sync diagnostics; simplified interview date saving and made participant name redaction toggle instantaneous.
- **Brand refresh and new app icon:** Updated Fleuron icon across macOS dock/finder bundles, Windows packaging, and in-app assets to the refreshed Floral-F identity.

## [2.4.1]

### Fixed

- **Passage note positioning and dark contrast:** Notes open beside the reading column on a solid, opaque card with a 1px rim border, never covering the passage text they quote; Esc closes pinned notes and returns focus to the trigger row.
- **Stripe and pill alignment:** Widened the stripe gutter to 28px and inset passage copy and pills footer to 32px (`pl-8`), preventing the 5th stripe from clipping into or overlapping pill buttons.
- **Filter place restoration:** Preserved pre-filter scroll fraction and selected passage across active filter states so list contraction cannot overwrite the reference place with 0, restoring the exact passage place on filter clear.

### Added

- **Single-click coding selection:** Clicking a highlighted phrase selects that exact coding and opens the coding bubble directly, eliminating the need to re-drag overlapping text.
- **Pill editing and dedicated filter controls:** Clicking a code pill selects the coding and opens the bubble; filtering moves to an explicit filter icon and Alt-click shortcut.
- **Span boundary legibility:** Adjacent coded spans now feature 2px code-color inset underlines and a 4px visual seam to distinguish touching highlights.
- **Filter count & empty state:** Active filters display "Showing X of Y passages" with a "Clear all" control; empty filter results display a dedicated no-match state.

## [2.4.0]

### Added

- **Shift+click extends the passage selection:** dragging across a phrase overshoots by a word and no longer means re-dragging — first click or drag sets an anchor, Shift+click extends to that point inside the same speaker turn.
- **Filtering keeps your place in the transcript:** applying or clearing a code/speaker filter keeps the selected passage visible when it survives, otherwise restores the same scroll fraction instead of jumping to the top.
- **Passage text zoom:** minus/percentage/plus control and Cmd/Ctrl +/−/0 scale only the reading serif (75–200%), persisted per machine.
- **Collapsible codebook:** a Hide button, slim expand rail, and Cmd/Ctrl+B give the transcript the full width; the collapsed choice and the previous width persist.
- **Bulk transcript import:** pick many files, check the suggested participant IDs on one review screen (duplicates blocked with the next-free ID offered), import once with per-file results; the review screen also links the folder-scan matcher.
- **Per-interview speaker redaction:** an Interview-settings toggle maps real names to Speaker 1, Speaker 2 in first-appearance order across display, copy actions, the speaker filter, and the CSV export. Stored transcripts and sync payloads are unchanged.
- **Manual QA checklist artifact:** every Windows QA run now writes a fill-in `manual-rows-{version}.md` for the five hands-on rows.
- **Codebook search polish:** search covers names, definitions, and criteria text with a match count and clear button.

## [2.3.0]

### Added

- **Two-section home layout:** The home screen clearly separates "On this computer only" from "Shared with a group", eliminating ambiguity over which studies sync and which remain strictly local.
- **Collaborative vocabulary:** Consistent, plain-language terminology across all menus, dialogs, and alerts ("Share with a group", "Stop syncing on this computer", "Leave group", "Delete group for everyone", "Delete from this computer").
- **Folder collision and join target inspection:** The join wizard inspects candidate folder names before creation, detects adoptable unbound studies, and generates clean disambiguated names when collisions exist.
- **Transcript candidate proposal and verification:** In the join wizard, candidate transcript files are automatically matched and compared against remote content hashes, verifying transcript identity before coding starts.
- **Same-titled study disambiguation:** Studies sharing the same title are flagged with clear warning banners and last-opened timestamps to differentiate separate project folders.
- **Duplicate share guard:** Attempting to share a study with the same name as an existing group warns the user and offers to connect to the existing group instead.
- **Sole-member leave guard:** Leaving a group as the only remaining member requires typing the full study title to confirm, preventing permanent loss of access to cloud data.
- **Access lost handling:** If access to a group is revoked or deleted on the server, local coding and project files are preserved and converted back to a standalone local study.
- **Study location resolver & cloud eviction detection:** Accurately diagnoses moved, missing, or cloud-evicted files (0-byte/cloud stubs) with clear explanations and re-download guidance.
- **Advisory concurrent open markers:** Detects when a study folder on a shared or cloud drive is actively open on another machine, warning before conflicting changes occur.
- **Study readiness checking:** Checks study folder readiness in the background, surfacing missing transcripts or unreadable states on project cards.
- **Two-machine QA test suites:** Added `scripts/Invoke-FleuronCollabQA.ps1` (pure ASCII) and `scripts/collab-qa.sh` covering end-to-end collaboration scenarios.

## [2.2.0]

### Added

- **Find-in-place transcript search:** Searching in the transcript panel now highlights occurrences in place without filtering non-matching passages away. Pressing `⌘F` (macOS) or `Ctrl+F` (Windows) focuses the search input. Matches can be navigated sequentially via `Enter` / `Shift+Enter` or `⌘G` / `F3` (`⇧⌘G` / `⇧F3`), with a live match counter ("3 of 17") in the sticky header and automatic scrolling to the active match.
- **Dedicated speaker filter:** Added a Speakers section to the transcript filter menu listing distinct speakers in the current transcript, composed with code filtering as `AND`.

### Improved

- **Passage rendering performance:** Repaired memoization dependencies in `TranscriptPanel` and memoized `PassageText` components, reducing typing frame latency during rapid coding by ~50% on large transcripts.

### Infrastructure

- **Multi-baseline Windows QA runner:** Parameterized `qa/Invoke-FleuronQA.ps1` to test both in-brand Fleuron (2.0.0 → candidate) upgrade transactions and legacy Codemap (1.2.0 → candidate) rebrand migration paths.
- **Strict stripe assertions:** Hardened `e2e/highlight-stripes.spec.ts` to test coded marks and gutter stripe geometry unconditionally across search states.

## [2.1.0]

### Added

- **In-app diagnostic report:** In About modal, users can generate a comprehensive plain-text diagnostic report covering application version, install integrity (poisoned nested directory detection), library project counts, sync status, storage health (WAL/SHM and stranded staging DB detection), updater state, and redacted crash logs. A mandatory scrollable preview is displayed before copying or saving to file.
- **External read-only diagnostic probe:** Added `support/` directory with `support/fleuron-probe.sh` (macOS) and `support/Get-FleuronProbe.ps1` (Windows PowerShell 5.1/pwsh) for users whose app fails to start. Both probes are strictly read-only, make zero network calls, and enforce the same 120-character allowlist redaction rule on crash payloads.
- **Diagnostics intake:** Updated install-help issue template with an optional diagnostic report field and linked troubleshooting instructions in installation and support guides.

### Infrastructure

- Clarified separation between user diagnostics (`support/` and in-app) and destructive maintainer test suites (`qa/`). The QA runners remain maintainer-only and are never published as release assets.

## [2.0.1]

### Fixed

- Backup restore retries the final database replacement when Windows briefly
  holds an SQLite handle, removes failed staging files, and reports actionable
  WAL/SHM diagnostics without deleting the live database.
- Windows installer QA distinguishes a healthy silent install from its required
  zero exit code and exercises the updater timeout-abort path before the real
  updater transaction.

### Infrastructure

- The QA runners are maintainer tools and are deliberately **not** published as
  release assets. Each one begins by wiping the default study library
  (`~/Fleuron` / `%USERPROFILE%\Fleuron`) with no confirmation prompt, so a
  release page is the wrong place to offer them. To verify a specific release,
  check out its tag and use the `qa/` directory from that commit. A contract
  test fails the build if a runner archive is added back to the release
  inventory or advertised in the release notes.
- The release inventory is therefore six assets, not eight.

## [2.0.0]

Codemap is now **Fleuron**.

This is a rename, not a rewrite — the app, its data model, and its sync
protocol are unchanged. It is a major version because the application
identifier changed, which means 2.0.0 installs alongside a 1.x install rather
than updating it.

### Changed

- Projects are now created as `.fleuron` folders. Existing `.codemap` and
  `.qcproj` projects open unchanged.
- The application data directory moved from `app.codemap.desktop` to
  `study.fleuron.desktop`. A 1.x install's recent-projects list, preferences,
  and signed-in session do not carry over; sign in again after upgrading.
- Backups are written as `.fleuronbak`. Existing `.codemapbak` files still import.
- The default library for new projects moved from `~/Codemap` to `~/Fleuron`
  (`%USERPROFILE%\Fleuron` on Windows). Projects already on disk are never moved.
- Build and CI environment variables are now `FLEURON_*`.

## [1.2.0] - 2026-08-27

### Added

- **Trust & permissions center** in-app: Files & permissions, Local vs collaboration, build provenance, and system warning disclosure in one reachable surface.
- **Collaboration disclosure** on onboarding's collaborate card, Join Study, and account creation: names exactly what leaves the machine — account email plus study, codebook, and coding records — with a link to the privacy guide; transcript text and memos always stay local.
- **Quiet update controls**: an automatic-checks toggle in settings; automatic up-to-date/failure outcomes are silent, manual failure and available updates stay visible.
- **Intent-first local onboarding**: create and code in a local study with no account; credentials and collaboration only enter once you choose collaboration.
- **Windows DPAPI storage** for the refresh token (`session.dpapi`, current-user DPAPI); legacy plaintext `session.json` migrates silently then deletes.
- **Public trust documentation**: nontechnical install guidance (`docs/INSTALLING.md`), exact privacy and permission disclosure (`docs/PRIVACY-AND-PERMISSIONS.md`), Support, Security, and IT-deployment guides for managed machines, signing activation notes, and install/bug-report issue templates.

### Changed

- **New projects default to a personal library** (`~/Codemap` on macOS, `%USERPROFILE%\Codemap` on Windows) instead of the protected Documents folder; existing paths are preserved and unavailable recents stay visible.
- **Folder access is consent at use**: native file/folder pickers are the consent; a denied picker shows a truthful inline choose-another-folder / how-to-fix state instead of retrying; the folder-scan disclosure appears before the scan; no Full Disk Access recommendation.
- **One restrained confirmation system** replaces scattered browser and app-owned prompts for destructive or destination choices.
- **macOS bundle** is sealed as a complete bundle with explicit ad-hoc signing and truthful file-purpose strings — Codemap is ad-hoc signed and un-notarized (no Apple Developer ID), so first launch legitimately triggers Gatekeeper with documented Open Anyway guidance.
- **Windows installer contract** is documented and machine-checked: current-user install, no elevation/prompt, SmartScreen / Unknown publisher / Controlled Folder Access / WebView2 / Mark-of-the-Web behavior in plain language.

### Infrastructure

- Rust CI now runs on `macos-latest` **and** `windows-latest`. A release-contract suite (49 tests) pins packaging and trust invariants, including fail-closed presence of the real-app selftest in every platform job with a negative fixture.
- New manual **candidate pipeline** (`candidate.yml`, non-publishing); the release workflow is draft-only with a provenance/attestation job and exact-asset inventory.
- The packaged real-app `--selftest` now runs in every candidate and release platform job and fails the run on any failure. The in-app selftest gained five suites — study lifecycle, transcript import, coding roundtrip, export artifacts, backup-restore — for 11 pass / 1 skip.
- Playwright E2E now triggers on **every push to `main`**; four deviant-path specs and a seeded monkey pass were added, and the dev mock gained `sync-error`, `server-conflict`, `auth-error`, and `slow` fault fixtures.
- Rust: `salvage_legacy_v1_state_for_v2` now keys salvaged coding per code (a migrated Protocol-1 span carrying two codes produces two correct entities), and backup staging filenames are UUID-suffixed so concurrent snapshots cannot collide.
- Server migration verification scripts (static + negative-fixture self-check) are restored as CI gates; this version makes no server or schema change (schema stays at 10).

## [1.1.0] - 2026-08-26

### Added

- **Real-time collaboration status** on the Study & sync sheet: an "active" chip when collaborative sync is on, a plain-language "turns on automatically" note while the team catches up, and a friendly message when sync is unavailable.
- **Beta disclosure** on account creation and the Study & sync sheet (and README): hosted sync is free while Codemap is in beta, becomes subscription-based afterwards, local/offline tools stay free, and beta joiners keep founder pricing.
- **Per-user sync entitlement substrate** on the server (dormant during the free beta — every account is entitled).

### Changed

- **Sync Protocol 2 activates automatically** for new and existing eligible studies: no button, no confirmation, no protocol jargon. The server's own readiness gate (every current member updated and synced once) still decides when; a not-ready study keeps working on the previous protocol until then.
- **Error mapping** for subscription-gated sync paths: create, join, redeem, and push on a lapsed account show one clear friendly message instead of generic permission or network text.
- **Protocol internals** (protocol number, generation, server head) moved into the collapsed Technical details block of the Sync Diagnostics panel, kept for support only.

### Infrastructure

- One additive migration adds the entitlement store, beta auto-grant + backfill, and the write-path gate; server schema certification stays at 10 and the app remains fully usable offline.
- Static migration verification and negative-fixture self-check restored, wired into CI; the release workflow now runs a fail-closed `validate` job (version identity, migration verification, typecheck, unit tests, build, Rust format/clippy/tests) before any platform build.
- Local Supabase verification scripts restored (`verify-supabase-migrations.sh`, `test-local-supabase-v2.sh`, `check-server-schema.sh`).

---

## [1.0.0] - 2026-08-25

### Initial Public Release

Codemap 1.0.0 is the first public, open-source release of the desktop qualitative coding environment for reflexive thematic analysis.

#### Features

- **Offline-First Core**: Fully functional standalone desktop workspace backed by local SQLite storage (`.codemap` project directories).
- **Interactive Coding**:
  - Drag-to-select phrase highlight coding with contextual floating action bubbles.
  - Whole-turn passage coding with intuitive toggle interactions.
  - Multi-code overlap rendering with customizable color palettes.
- **Living Codebook**:
  - Full codebook management: create, rename, recolor, merge, split, and retire codes.
  - Interactive passage drawer showing all quotes assigned to a code in real time.
- **Transcript Parsing & Ingestion**:
  - Automatic format detection and parsing for WebVTT (`.vtt`), SubRip (`.srt`), speaker-labeled plain text (`.txt`), Microsoft Word (`.docx`), and tabular transcripts (`.csv`).
- **Reflexive Memoing**:
  - Highlighting-level notes and interview-wide memos preserved locally on disk.
- **Multi-Coder Sync (Protocol v2)**:
  - Optional multi-user synchronization via self-hosted or managed Supabase backend.
  - Strict privacy boundary: code definitions and coded segment hashes sync while transcript texts and private memos stay strictly local.
- **Export Formats**:
  - Export coded passages and codebooks to CSV, formatted Markdown, standalone HTML summaries, and DOCX documents.
- **Platform Support**:
  - Native builds and installers for macOS (Apple Silicon and Intel) and Windows 10/11.
