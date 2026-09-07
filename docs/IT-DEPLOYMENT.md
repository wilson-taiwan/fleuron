# IT deployment guide

Fleuron is a solo-maintained, open-source desktop application for qualitative coding. This page tells IT teams exactly what it installs, what it touches, and which controls decide whether it may run.

## Support matrix

| Tier | Platforms |
| --- | --- |
| Supported | macOS 14+ (universal Intel + Apple Silicon); Windows 11 x64 on a standard personal machine/user account |
| Best effort | macOS 10.15–13; Windows 10 (out of support since 2025-10-14) |
| Not supported | Windows ARM, S mode, virtualized edge cases; managed devices must be allow-listed by your own policy |

## Installer facts (Windows)

- Single x64 NSIS installer: `Fleuron_2.0.1_x64-setup.exe`
- **Current-user install only** (`RequestExecutionLevel user`, `installMode: currentUser`). No UAC prompt, no per-machine option, no elevation anywhere in install/update/uninstall
- Installs beneath the user's LocalAppData per-user program path
- **WebView2:** `webviewInstallMode: skip`. Fleuron never downloads, bundles, installs, or repairs WebView2. Windows 11 normally ships Evergreen already

## Package provenance

- No Authenticode certificate is purchased; SmartScreen shows *Unknown publisher → Run anyway* for first runs
- macOS builds are ad-hoc signed whole bundles: `codesign --verify --deep --strict` passes with `Signature=adhoc`, `TeamIdentifier=not set`. Gatekeeper still warns and users must use System Settings → Privacy & Security → Open Anyway once
- Every release includes GitHub artifact attestations and a sorted `SHA256SUMS.txt`; verify with:

```bash
gh attestation verify Fleuron_2.0.1_x64-setup.exe --repo wilsonhyeh/fleuron
shasum -a 256 -c SHA256SUMS.txt   # or sha256sum -c on Linux
```

Hash-per-release allowlisting fits this model cleanly: pin the published SHA-256 digest of the exact installer per release.

## Network

- Outbound only: HTTPS to api.github.com / github.com for update checks + downloads (turn off via Settings → Update checks), and HTTPS/WSS to the configured Supabase host when users collaborate
- No inbound listener; no firewall exception required or ever created by us
- Nothing else — no telemetry, no crash upload, no analytics

## Files and data locations

| What | Where |
| --- | --- |
| Default study library | `%USERPROFILE%\Fleuron` / `~/Fleuron` |
| App preferences | OS app-data dir (`app-preferences.json`) |
| Recents/membership caches | Same app-data dir |
| Stored sign-in | Windows: DPAPI blob `session.dpapi` (current user). macOS: mode-0600 `session.json` |
| Project data | A local SQLite database inside each study folder |

Controlled Folder Access: studies default outside protected folders. If users choose protected folders deliberately, treat CFA denials as normal app behavior — the app offers choose-another-folder inline. We never suggest disabling CFA.

## Policy blocks are respected

Smart App Control, WDAC/App Control for Business, and AppLocker blocks have no consumer bypass and we will not provide one. If you allow-list Fleuron, allow the specific signed bundle ID (`study.fleuron.desktop` on macOS) and/or hash-pinned installer per release.

## Update behavior

GitHub Releases latest.json drives the updater channel. Updates download only after explicit user action and install via a per-user restart flow without elevation. Pinning strategy: disable automatic checks in Settings and deploy exact installers from your pipeline if you need fixed versions.

## Contact

Security issues: GitHub private vulnerability reporting ([SECURITY.md](../SECURITY.md)). Install/deployment problems: GitHub Issues with the install-help form.
