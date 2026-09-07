# Fleuron Windows VM QA Runner

Automated QA test runner for verifying Fleuron releases on Windows 11 virtual machines.

## Overview

The QA runner is written in pure PowerShell 5.1 (standard with Windows 11) with zero runtime dependencies (no Node, npm, or Git needed in the VM). It performs an unattended verification pass across fresh installation, installed-binary selftests, crash sweeps, legacy project compatibility, and updater transactions, followed by a numbered manual checklist.

## Release asset

**This runner is a maintainer tool and is deliberately not published as a release asset.** It wipes `%USERPROFILE%\Fleuron` -- the default study library -- along with all app state, with no confirmation prompt. Never run it on a machine holding real coding.

To verify a specific release, check out that release's tag and use the `qa/` directory from that commit:

```
https://github.com/wilsonhyeh/fleuron/archive/refs/tags/v<version>.zip
```

The runner derives the expected version from a canonical `Fleuron_X.Y.Z_x64-setup.exe` filename, or accepts `-ExpectedVersion X.Y.Z`. (A `release.json` beside the script also pins it, but nothing generates one any more.)

## Quick Start (VM Execution)

1. **Copy runner to VM:** Drag the repository's `qa/` folder into your Windows 11 VM. Use a throwaway VM snapshot -- leg 1 wipes all Fleuron and Codemap state, including the study library.
2. **Obtain Candidate Installer:** Place the matching `Fleuron_X.Y.Z_x64-setup.exe` in the VM.
3. **Run in PowerShell:**
   ```powershell
   powershell -ExecutionPolicy Bypass -File .\Fleuron-Windows-QA-Runner\Invoke-FleuronQA.ps1 -Candidate .\Fleuron_X.Y.Z_x64-setup.exe
   ```
4. **Review Evidence:**
   - Open `qa-evidence\SUMMARY.md` for the test table and diagnostics.
   - Complete the numbered items under `## MANUAL — <N> rows for you`.
   - Drag `qa-evidence\` back to the host machine for records.

## CLI Arguments

| Parameter | Description | Default |
|---|---|---|
| `-Candidate <path>` | Path to the candidate installer executable (`Fleuron_X.Y.Z_x64-setup.exe`). | Mandatory |
| `-ExpectedVersion <ver>` | Expected candidate version when the filename is noncanonical; release assets normally resolve this from `release.json`. | Optional |
| `-Previous <path>` | Path to the previous Codemap release installer (`Codemap_1.2.0_x64-setup.exe`). Used for the Codemap rebrand updater leg. | Optional (local copy needed for Codemap leg) |
| `-PreviousVersion <ver>` | Version tag of the Codemap baseline installer. | `"1.2.0"` |
| `-PreviousFleuron <path>` | Path to previous Fleuron baseline installer (`Fleuron_2.0.0_x64-setup.exe`). If omitted, runner auto-downloads from GitHub. | Auto-download |
| `-PreviousFleuronVersion <ver>` | Version tag of the Fleuron baseline installer. | `"2.0.0"` |
| `-OutputDirectory <path>` | Target folder for `SUMMARY.md`, `evidence.json`, and `raw/`. Rotates existing folders if present. | `qa-evidence` |
| `-Online` | Opt-in switch to run cloud/sync selftests against staging Supabase. Requires `qa.env.ps1` in the runner folder. | Disabled |
| `-SkipUpdater` | Skip the simulated updater transaction legs. | Disabled |
| `-SelfCheck` | Run automated self-checks of runner modules against offline fixtures (used by CI). | Disabled |

## Test Legs

1. **Wipe (`wipe`):** Provably clears all Fleuron and Codemap AppData, LocalAppData, and Registry uninstall entries, asserting 0 survivors. Sets up synthetic canaries to detect accidental data loss.
2. **Fresh Install (`fresh_install`):** Installs candidate silently (`/S`), verifies binary attributes, shortcuts, registry keys, and runs the installed executable's `--selftest` suite (12 suites).
3. **Crash Sweep (`crash_sweep`):** Scans `%APPDATA%\study.fleuron.desktop\crashes\crash.log`, `%LOCALAPPDATA%\CrashDumps\*.dmp`, and Windows Error Reporting (WER) logs.
4. **Relaunch (`relaunch`):** Confirms installed binary boots cleanly and closes without hanging.
5. **Legacy Projects (`legacy`):** Synthesizes flat and nested `.codemap` study folders under paths with spaces and verifies command-line opening.
6. **Updater Simulation (`updater`):** Parameterized multi-baseline updater verification testing two distinct migration tracks:
   - **Codemap 1.2.0 Rebrand Baseline (`codemap_120`):** Installs Codemap 1.2.0, seeds user project data under `app.codemap.desktop`, and drives candidate upgrade. Asserts Codemap.exe is replaced/removed and data preserved. (Requires local `-Previous` because Codemap releases are archived).
   - **Fleuron 2.0.0 Baseline (`fleuron_200`):** Installs Fleuron 2.0.0 (auto-downloaded or via `-PreviousFleuron`), seeds data under `study.fleuron.desktop`, and tests in-brand upgrade transaction to candidate version.
7. **Online (`online`):** Dot-sources `qa.env.ps1`, passes `--require-online`, and tests real-time collaboration and DPAPI token persistence across restarts.

## Online Staging Setup (`qa.env.ps1` in the runner folder)

When using `-Online`, create `qa.env.ps1` beside `Invoke-FleuronQA.ps1` (gitignored in a source checkout) with the following environment variables:

```powershell
$env:FLEURON_STAGING_SUPABASE_URL = "https://your-project.supabase.co"
$env:FLEURON_STAGING_SUPABASE_ANON_KEY = "your-anon-key"
$env:FLEURON_STAGING_OWNER_EMAIL = "owner@example.com"
$env:FLEURON_STAGING_OWNER_PASSWORD = "password"
$env:FLEURON_STAGING_JOINER_EMAIL = "joiner@example.com"
$env:FLEURON_STAGING_JOINER_PASSWORD = "password"
```
