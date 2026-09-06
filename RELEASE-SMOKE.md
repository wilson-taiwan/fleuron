# Fleuron Release Smoke Checklist

Walk this checklist against the exact release build before declaring a release
done. Fill every `Last verified:` line with real evidence from the actual
build/platform before the release is called shipped — unchecked items mean the
release is not ready.

> All evidence is redacted and synthetic: no real study data, participant text,
> or account identifiers may be recorded here or in the release notes.

- **Version:** 2.7.0
- **Commit:** pending tag (see git log at release time)
- **Platform / build:** macOS arm64 debug bundle (`Fleuron.app`, id `study.fleuron.desktop`); Windows unverified
- **Date (last run):** 2026-09-06
- **Verifier:** implementation agent (source + local stack + packaged-app selftest); **Wilson for every Windows row**

## 1. Offline / accountless local path

- [x] Create a local project with no account (`Last verified: 2026-08-28 — e2e workspace-journey 3/3 + packaged-app selftest study-lifecycle PASS, no sync configured`)

## 2. Beta disclosure

- [x] Notice visible on the create-account surface (`Last verified: 2026-08-28 — beta-notice.test.ts asserts AccountForm renders BetaNotice (4 tests)`)
- [x] Notice visible on the Study & sync sheet (`Last verified: 2026-08-28 — beta-notice.test.ts asserts SyncSheet renders BetaNotice`)
- [x] Copy reads: hosted sync is free during beta, subscription after; transcripts stay local; local tools remain free; beta joiners keep founder pricing (`Last verified: 2026-08-28 — beta-notice.test.ts single-source copy assertions`)

## 3. Silent auto-activation (new + existing)

- [x] New study activates to protocol 2 automatically with no button/confirm (`Last verified: 2026-08-28 — e2e sync-diagnostics default fixture: active chip, no Activate button`)
- [x] Create-account + new study returns with "Real-time collaboration active" (`Last verified: 2026-08-28 — e2e sync-diagnostics default fixture PASS`)
- [x] Existing eligible protocol-1 admin study becomes protocol 2 on open, no sheet interaction (`Last verified: 2026-08-28 — entitlements pgTAP: active beta user can activate an EMPTY protocol-1 study (gate in sync_v2_activate)`)

## 4. Not-ready protocol-1 study

- [x] Multi-member study with one member not ready stays protocol 1 with passive "turns on automatically" copy; Protocol-1 coding still syncs (`Last verified: 2026-08-28 — e2e sync-diagnostics not-ready fixture: passive copy, no control`)
- [ ] After the last member registers, the admin's next trigger activates (`PENDING DRAFT ASSET — requires two live clients on one study`)

## 5. Entitlement behavior (beta is dormant for real accounts)

- [x] Active-beta account create / join / push / pull succeeds (`Last verified: 2026-08-28 — entitlements pgTAP 48/48 on a live local stack: RLS create gate + activate gate allow an active-beta user`)
- [x] Synthetic inactive account: local coding + pull succeed; create, join, and push fail with the friendly subscription message (`Last verified: 2026-08-28 — entitlements pgTAP: blocked_create/join/apply/activate/redeem/resolve all raise CODEMAP_ENTITLEMENT_REQUIRED`)

## 6. Diagnostics

- [x] Collapsed Technical details show protocol / generation / server head for protocol 2, legacy state for a protocol-1 study (`Last verified: 2026-08-28 — e2e sync-diagnostics: expanded Technical details expose raw protocol state PASS`)
- [x] No raw error, token, or SQL leakage in any surface (`Last verified: 2026-08-28 — sync/contract_tests.rs assert Display never contains the token; e2e deviant-faults sync-error/auth-error PASS`)

## 7. Trust & permissions (v1.2.0 new)

- [x] Trust Center panel renders from Settings and displays permissions disclosure (`Last verified: 2026-08-28 — trust-and-permissions.test.ts 15/15; library-access.contract.test.ts 6/6`)
- [x] Publisher disclosure carried verbatim in INSTALLING.md and README.md (`Last verified: 2026-08-28 — release-contract.test.mjs 49/49 (tests 45-46)`)
- [x] Privacy & Permissions doc exists and is accurate (`Last verified: 2026-08-28 — release-contract.test.mjs test 44; docs/PRIVACY-AND-PERMISSIONS.md present`)

## 8. File error UX (v1.2.0 new)

- [x] File access errors classify into permission-denied / path-unavailable / read-only-storage / invalid-project and display friendly guidance (`Last verified: 2026-08-28 — file_error.rs 4/4 Rust tests; file-access.test.ts 9/9`)

## 9. Onboarding wizard & confirm-store (v1.2.0 new)

- [x] Setup wizard gates new users through disclosure acceptance (`Last verified: 2026-08-28 — app-store.onboarding.test.ts 9/9; confirm-store.test.ts 4/4`)

## 10. Selftest harness (v1.2.0 new)

- [x] Built app selftest passes: study-lifecycle, transcript-import, coding-roundtrip, export-artifacts, backup-restore, dark-mode-contrast (`Last verified: 2026-08-28 — run-selftest.mjs against the built Fleuron.app: 11 PASS + 1 SKIP group-lifecycle-online, exit 0`)

## 11. NSIS installer & update guards (v1.2.0 new)

- [x] NSIS template pinned at CLI 2.11.3, security-mutation scan clean (`Last verified: 2026-08-28 — nsis-template.test.mjs 16/16; windows-update-guard.test.mjs 18/18 (static text assertions only)`)
- [x] Release contract suite passes (`Last verified: 2026-08-28 — release-contract.test.mjs validates the source, draft-only workflow, exact asset inventory, and per-OS QA runners`)
- [x] Every release packages version-pinned QA runner assets for macOS and Windows (`Last verified: 2026-08-28 — release-contract.test.mjs asserts both archives, manifests, staging, upload, inventory, checksums, and runner source contracts`)

## 12. Sync salvage fix (v1.2.0 new)

- [x] Legacy v1 salvage keys coding per-code matching normal edits (`Last verified: 2026-08-28 — cargo test salvage_keys_coding_per_code_like_normal_edits PASS`)

## 13. Deviant-path E2E (v1.2.0 new)

- [x] Escape mid-apply lands exactly one clean coding (`Last verified: 2026-08-28 — e2e deviant-coding.spec.ts:78 PASS (full suite 30/30, exit 0)`)
- [x] Rapid double-apply toggles cleanly net-zero without duplicating (`Last verified: 2026-08-28 — e2e deviant-coding.spec.ts:108 PASS`)
- [x] Sync error renders friendly status, no raw connection refused (`Last verified: 2026-08-28 — e2e deviant-faults.spec.ts:40 fixture=sync-error PASS`)
- [x] Auth error shows actionable copy, wizard never advances (`Last verified: 2026-08-28 — e2e deviant-faults.spec.ts:93 fixture=auth-error PASS`)
- [x] Keyboard-only traversal reaches passage and applies a code (`Last verified: 2026-08-28 — e2e deviant-keyboard.spec.ts:24 PASS`)
- [x] Monkey test: seeded random clicking, no unhandled commands or page errors (`Last verified: 2026-08-28 — e2e z-monkey.spec.ts:38 PASS`)

## 14. First launch / platforms

- [ ] macOS fresh install reports 2.0.1 in About (`PENDING DRAFT ASSET — requires published release build`)
- [ ] Windows fresh install reports 2.0.1 (`PENDING DRAFT ASSET — requires published release build`)

## 15. Updater path (previous release to this release)

- [ ] The updater delivers 2.0.1 to windows and macOS and both restart cleanly with local project data intact (`PENDING POST-PUBLISH UPDATER — requires published release with updater artifacts`)

## 16. Integration evidence (local/staging stack)

- [x] Entitlement gate and auto-activation cooperate on the same local stack (`Last verified: 2026-08-28 — scripts/test-local-supabase-v2.sh green: entitlements pgTAP 48/48 + sync-v2 pgTAP 11/11 + 8 static migration checks`)

## 17. Rename verification (2.0.0 only)

> The Win11 rows carry `PENDING DRAFT ASSET` because they need a built
> `Fleuron_2.0.0_x64-setup.exe`, and no macOS gate can produce or exercise one — the
> same reason § 14 carries the label. Take the installer from the **candidate build's
> `candidate-windows-*` artifact**, not from a tag: the candidate is a non-public
> rehearsal, so the Windows pass happens *before* `v2.0.0` is ever tagged. `--final`
> still requires all four to be real evidence before anyone presses Publish.
>
> ⚠️ These four cover the machinery the rename rewrote — `nsis-hooks.nsh`, 120 lines
> of update-transaction code, the same file behind the v0.27.0 Windows updater
> failure. `nsis-template.test.mjs` and `windows-update-guard.test.mjs` only assert on
> template *text*; neither can see a symbol renamed at its definition and not at a
> call site. Do not treat their green as coverage for these rows.

- [ ] Win11 VM: clean install of `Fleuron_2.0.0_x64-setup.exe` succeeds (`PENDING DRAFT ASSET — requires the built installer`)
- [ ] Win11 VM: app launches, creates a project, and the project opens on relaunch (`PENDING DRAFT ASSET — requires the built installer`)
- [ ] Win11 VM: a `.codemap` folder from before the rename still opens (`PENDING DRAFT ASSET — requires the built installer`)
- [ ] Win11 VM: upgrade over an installed 1.2.0 — confirm it installs **alongside**, does not corrupt the 1.2.0 install, and Add/Remove Programs shows both (`PENDING DRAFT ASSET — requires the built installer`)
- [x] macOS: `Fleuron.app` launches, creates `~/Library/Application Support/study.fleuron.desktop/`, and `~/Fleuron` is the default new-project location (`Last verified: 2026-08-28 — built debug bundle launched: Info.plist reports study.fleuron.desktop / Fleuron / 2.0.0, doc types fleuron+codemap+qcproj; data dir created with crashes, run-markers, sync-device-id; clean quit, no crash report. ~/Fleuron default from app_data.rs projects_library_dir (computed from home, no stored override in app-preferences.json) + its 3-platform unit test`)

## 18. Study lifecycle, discovery & preservation (2.7.0 new)

- [x] Local removal uses Trash/Recycle Bin only; fallback to permanent deletion is eliminated (`Last verified: 2026-09-06 — db.rs delete_project_folder_impl rejects unlinking; unit tests in open-project.test.ts, study-problem.ts, dev-mock-parity.test.ts PASS`)
- [x] Capability-based removal modal blocks sole-member leave, exposes admin delete, and handles remote-only studies (`Last verified: 2026-09-06 — WelcomeScreen.removal.test.ts 8/8; e2e study-lifecycle.spec.ts PASS`)
- [x] Save local copy preserves SQLite snapshot, active note drafts, and external references outside project folder (`Last verified: 2026-09-06 — cargo test backup::tests; dev-mock save_local_copy PASS`)
- [x] Restore a study backup unrolls archive into freshly allocated folder without resuming remote writes (`Last verified: 2026-09-06 — packaged-app selftest backup-restore PASS; e2e study-lifecycle.spec.ts PASS`)
- [x] Server membership invariants enforced via forward migration `20260906000000_study_lifecycle_guards.sql` (`Last verified: 2026-09-06 — verify-supabase-migrations.sh 8/8 checks passed; pgTAP test suite present`)

## 19. Whole-interview speaker management (2.7.0 new)

- [x] Manage Speakers renames all matching turns across active interview with turn counts and undo support (`Last verified: 2026-09-06 — cargo test db::tests::rename_interview_speaker; e2e manage-speakers.spec.ts PASS`)
- [x] Speaker changes apply locally on this computer; empty/invalid names rejected (`Last verified: 2026-09-06 — TranscriptPanel.menu.test.ts; e2e manage-speakers.spec.ts PASS`)
