# Installing Fleuron

> **Publisher verification notice**
> Fleuron is an independent open-source application. This build does not yet carry an Apple Developer ID/notarization or Windows Authenticode publisher signature, so your operating system cannot verify its publisher automatically. Download only from `https://fleuron.study` or the official release page at `https://github.com/wilsonhyeh/fleuron/releases`. Continue only when the version, filename, and warning match this guide. A malware warning, checksum mismatch, or unexpected administrator request means stop.

## Choose your file

Fleuron is distributed from `https://fleuron.study` and the [official Releases page](https://github.com/wilsonhyeh/fleuron/releases); website buttons link directly to binaries hosted on that release page.
Only these two files are manual downloads:

| Your computer | Download this exact file |
| --- | --- |
| macOS (Intel or Apple Silicon) | `Fleuron_2.7.0_universal.dmg` |
| Windows 11 x64 | `Fleuron_2.7.0_x64-setup.exe` |

You may also see these on a release page. They are **updater infrastructure — do not download them by hand**:
`Fleuron_universal.app.tar.gz`, `.sig` signature files, `latest.json`, `SHA256SUMS.txt`.

## Three checks before you continue

1. **Official source:** you are on `https://fleuron.study` or `https://github.com/wilsonhyeh/fleuron/releases`, and the download link points to `github.com/wilsonhyeh/fleuron/releases`, not a mirror or ad link.
2. **Version and filename:** the file matches the exact name above for your platform.
3. **Expected warning:** the warning your OS shows matches the *expected* text below.

## Install on macOS

1. Open the DMG and drag **Fleuron** into **Applications**.
2. Launch Fleuron.
3. **Expected warning:** “Apple cannot check the app for malicious software” / “cannot verify the developer”. This release does not have Apple Developer ID signing or notarization. Open-source licensing does not determine whether an app is signed.
4. Continue: close the dialog, then open **System Settings → Privacy & Security**, scroll down and click **Open Anyway**, authenticate, then **Open**.

Open Anyway stays available after the first run unless macOS re-prompts after an update.

## Install on Windows 11

1. Double-click `Fleuron_2.7.0_x64-setup.exe`.
2. **Expected warning:** SmartScreen shows **“Windows protected your PC”** with *Unknown publisher*. Expected for a newly published, non-store app.
3. Click **More info**, confirm *Unknown publisher*, then **Run anyway**.
4. The installer installs for **your user account only**. It never asks for administrator credentials. There is no UAC prompt anywhere in this flow.

## Stop here if…

| You see | What it means | What to do |
| --- | --- | --- |
| macOS says the app is **damaged**, or “will damage your computer”, or a malware alert | Not the ordinary unsigned warning | Stop. Delete the download. Compare the SHA-256 digest below; if an official re-download repeats it, [file an install issue](https://github.com/wilsonhyeh/fleuron/issues/new?template=install-help.yml) from a safe device |
| Microsoft Defender reports a threat or quarantines the installer | Real security signal pending investigation | Stop. Do not restore or allow the file. Save the exact warning text and file an issue |
| A UAC / administrator prompt appears during install | Fleuron installs per-user; this should never happen | Cancel. Do not enter credentials. File an install issue |
| **Smart App Control** blocks the app | No per-app bypass exists | Stop. Do not disable Smart App Control |
| Windows **S mode** blocks installation | S mode runs Store apps only | Stop. Fleuron does not support S mode and we will not suggest switching out of it |
| WDAC / AppLocker / managed policy blocks the app | Organization-controlled device | Contact IT and share [docs/IT-DEPLOYMENT.md](IT-DEPLOYMENT.md) |

## File access troubleshooting

- New studies default to `~/Fleuron` (macOS) or `%USERPROFILE%\Fleuron` (Windows), which avoids protected Documents/Desktop folders.
- macOS may ask once about Documents/Desktop/Downloads/network or removable volumes **because you selected such a location yourself**. Allow, or cancel and choose another folder. Fleuron never requests Full Disk Access.
- If macOS Files & Folders access was denied by mistake: System Settings → Privacy & Security → Files and Folders → Fleuron → enable what you chose. Deleting data was never involved — Fleuron simply could not read there.
- **Controlled Folder Access** (Windows) blocked a save into a protected folder? Choose another folder; the default `%USERPROFILE%\Fleuron` is outside protected folders. Never disable CFA.

## WebView2 (rare)

Windows 11 normally includes WebView2. Fleuron never downloads, bundles, or repairs it. If it is missing or damaged, use Microsoft’s official Evergreen Runtime guidance — managed devices contact IT.

## Advanced verification

Optional steps. Normal installation never requires Terminal or checksums.

```bash
# GitHub's per-asset digest, visible on the release page:
shasum -a 256 Fleuron_2.4.0_universal.dmg
```

Verify against the published `SHA256SUMS.txt` on the release page:

```bash
gh release download v2.4.0 --repo wilsonhyeh/fleuron --dir .
shasum -a 256 -c SHA256SUMS.txt
```

Artifact attestation proves which public repository, workflow, and commit produced each asset:

```bash
gh attestation verify Fleuron_2.4.0_universal.dmg --repo wilsonhyeh/fleuron
```

These checks help verify file integrity and build provenance. They do not establish software safety or replace an OS publisher identity.

## Troubleshooting & diagnostics

If you encounter unexpected behavior or errors:

1. **In-app diagnostic report:** If Fleuron launches, open **About → Generate diagnostic report**. You can preview the full plain-text report, copy it, or save it to a file. It is designed to include allowlisted installation and sync metadata rather than study content. Review the entire report before sharing; do not include transcripts, participant identifiers, code names, credentials, or personal file paths.
2. **External read-only probe:** If the application fails to launch at all, run the standalone diagnostic probe from the `support/` directory in the repository:
   - macOS: `bash support/fleuron-probe.sh --output fleuron-diag.txt`
   - Windows: `powershell -NoProfile -File support\Get-FleuronProbe.ps1 -OutputFilePath fleuron-diag.txt`
   Both probes are strictly read-only and never modify your system or study files.

## Still stuck?

[Open an install-help issue](https://github.com/wilsonhyeh/fleuron/issues/new?template=install-help.yml). Do not attach transcripts, quotes, participant/study identifiers, project databases, tokens, or unredacted crash logs/screenshots.
