# Note reliability — verification summary (synthetic evidence only)

No real study data, participant text, or account identifiers are recorded here.
All projects referenced are synthetic (`seedFreshStudy`, browser dev-mock mirror).

Date: 2026-09-06 (UTC). Repo: `fleuron`, base `192ac24` v2.5.0, working tree uncommitted (no commit/tag/push per plan).

## Gate results (verbatim commands, from repo root)

| Command | Exit | Result |
|---|---|---|
| `npx tsc --noEmit` | 0 | clean |
| `npm test` | 0 | 56 files / 453 passed |
| `npm run build` | 0 | built in ~1.9s |
| `npm run test:e2e` | 0 | 54 passed (6.5m), incl. `note-draft-lifecycle` 13/13 and `workspace-notes` 3/3 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` | 0 | clean |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | 0 | clean |
| `FLEURON_FUZZ_BUDGET_SECS=3600 cargo test --manifest-path src-tauri/Cargo.toml` | 0 | 301 passed — see `fuzz-3600.log` |
| `node --test scripts/release-contract.test.mjs` | 0 | 55 passed |
| `bash scripts/verify-supabase-migrations.sh` | 0 | all migration checks passed |
| `bash scripts/verify-supabase-migrations.test.sh` | 0 | 8/8 self-checks passed |
| `bash scripts/verify-ps1-ascii.sh` | 0 | ASCII-only |
| `npm run tauri build -- --debug --no-bundle` | 0 | `src-tauri/target/debug/Fleuron` built |
| `node scripts/run-selftest.mjs --binary src-tauri/target/debug/Fleuron` | 0 | selftest passed — see `selftest-debug.log` |

## Coverage of the plan's failure modes

- False autosave success → interview autosave truthful (`Not saved` + Retry, no `Saved` on reject): e2e `note-commit-fail` specs + store tests.
- Lost newer revision (A→B during save A) → revision CAS + per-target serialization: Rust `note_recovery` tests + draft-store deferred-promise tests.
- Collapse loses draft → drafts retained, reopen restores: e2e collapse/reopen, filter hide/restore, interview-switch specs.
- Competing editors → single inline editor (`activeInlineCodingId`), rail removed: e2e convergence spec + selftest suite 17.
- Duplicate alerts → single `ToastStack` in `App.tsx`: e2e exactly-one-Notifications specs.
- Crash recovery → private `note-recovery.sqlite3`, acknowledged-write durability, commit-first/clear-after: Rust reopen/cleanup-failure/crash-boundary tests + selftest suites 14–16 + e2e recovery-panel spec.
- Departure/export → typed `save|discard|cancel` controller, native `notes://departure-requested` + one-use intent, committed-only export snapshot: departure-store tests (10) + updater regression tests (5) + e2e close-study specs + selftest clean-departure path.
- Privacy → markers live only in recovery; export/backup/sync carry no draft text: `note-privacy-contract` selftest suite + Rust `recovery_markers_reach_no_committed_pipeline`.

## Native smoke (macOS, debug binary)

- Fresh `tauri build --debug --no-bundle` binary launched, ran with empty log output, quit without a lingering process. (A desktop `screencapture` was taken to confirm paint but deliberately not archived — it captures the operator's desktop, not the app alone.)
- Real-IPC selftest (`--selftest`, isolated recovery root via `setRecoveryRootForSelftest`, synthetic projects only) passed, covering suites 14–17.

## Blocked / not claimed

- Windows close/Alt+F4, Win11 installer rows: no Windows access — not run, not claimed.
- Real updater install: never run without a synthetic candidate per plan; cancellation/failure paths covered by tests only.
- Forced OS termination between commit and recovery cleanup: covered by a simulated test, not a real `kill -9` of the operator's app.
- Full `RELEASE-SMOKE.md` rewrite: untouched — it belongs to released 2.5.0 and this plan is implementation-only with no release authorized. `node scripts/verify-release-smoke.mjs --final` not run (would fail on the pre-existing `PENDING DRAFT ASSET` external rows).
- Production sync/live stack: unchanged by design (no server/schema/protocol changes); migration guards green.
