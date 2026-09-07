# Fleuron Support Diagnostics

This directory contains lightweight, **read-only** diagnostic tools for troubleshooting installation or launch issues when the Fleuron desktop app cannot start.

If Fleuron starts normally, use the in-app diagnostic report instead: **About → Generate diagnostic report**.

## Guarantees

- **Strictly read-only:** These scripts never install, uninstall, delete, move, or modify files (other than writing to a single output file you specify).
- **Offline & private:** No network connections are made and no data is transmitted automatically.
- **Redacted data boundaries:** Transcripts, quotes, participant identifiers, study names, and codebook entries are strictly excluded. Any crash log payloads are truncated to a safe 120-character allowlist preview.

## Running the probe

### macOS

```bash
bash support/fleuron-probe.sh --output fleuron-diagnostic.txt
```

To view help:
```bash
bash support/fleuron-probe.sh --help
```

### Windows (PowerShell 5.1 or pwsh)

```powershell
powershell -NoProfile -File support\Get-FleuronProbe.ps1 -OutputFilePath fleuron-diagnostic.txt
```

To view help:
```powershell
powershell -NoProfile -File support\Get-FleuronProbe.ps1 -Help
```

## Review and Sharing

Always inspect the generated diagnostic text file before attaching it to a GitHub issue:
- Verify that only general system information and truncated crash logs are included.
- Attach the file to your [install-help issue](https://github.com/wilsonhyeh/fleuron/issues/new?template=install-help.yml).
