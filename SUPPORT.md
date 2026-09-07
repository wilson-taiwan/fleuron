# Support

Fleuron is maintained by one person, so support runs entirely through GitHub — no email, no chat.

## Install help or bugs

Open a GitHub Issue using the guided forms:

- **Install help:** [New install-help issue](https://github.com/wilsonhyeh/fleuron/issues/new?template=install-help.yml)
- **Bug report:** [New bug-report issue](https://github.com/wilsonhyeh/fleuron/issues/new?template=bug-report.yml)

Forms collect OS version/build, Fleuron version and build commit (About → Copy build details), the exact filename and download URL, and the exact warning text.

To provide diagnostics safely:
- **In-app:** About → **Generate diagnostic report** to copy or save a fully redacted summary of installation, storage, and sync state.
- **When the app will not start:** Run the standalone read-only probe from the repository's `support/` folder (`bash support/fleuron-probe.sh` on macOS or `powershell -NoProfile -File support\Get-FleuronProbe.ps1` on Windows).

**Before attaching anything:** do not upload transcripts, quotes, participant/study identifiers, project databases, tokens, private URLs, or unredacted crash logs/screenshots. Crash logs may contain file paths — review and redact first.

## Security vulnerabilities

Please report privately through GitHub's **Report a vulnerability** flow on the repository ([security advisories → report](https://github.com/wilsonhyeh/fleuron/security)). See [SECURITY.md](SECURITY.md). Do not open public issues for security reports.

## Deployment questions for IT

Start with [docs/IT-DEPLOYMENT.md](docs/IT-DEPLOYMENT.md), then Issues if something remains unclear.

## Documentation

- [Installing](docs/INSTALLING.md) ([fleuron.study/install](https://fleuron.study/install)) — canonical downloads + expected OS warnings
- [Privacy and permissions](docs/PRIVACY-AND-PERMISSIONS.md) ([fleuron.study/privacy](https://fleuron.study/privacy)) — exact data boundary
