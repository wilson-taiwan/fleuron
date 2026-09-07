# Fleuron

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

Offline-first desktop application for **reflexive thematic analysis**. Import transcripts, code passages (whole turns or phrase selections) with a living codebook, write reflexive memos, and collaborate with your research team across machines.

Built with **Tauri v2**, **React 19**, **TypeScript**, **Tailwind CSS**, and **SQLite**.

---

## Key Features

- **Offline-First Privacy**: Project data is stored in a local SQLite database (`.fleuron`). Collaboration sync excludes transcript text and memo fields. Shared exports and cloud-synced folders can take local files off the device.
- **Span & Whole-Turn Coding**: Drag across words to highlight a specific phrase or click a passage to code the whole turn.
- **Living Codebook**: Create, merge, split, recode, color-code, and organize codes hierarchically. Click any code to inspect all associated passages.
- **Collaborative Sync (Optional)**: Connect via self-hosted or managed Supabase using Sync Protocol v2. Coded spans, codebook structure, memberships, and your de-identified labels sync over encrypted transport; transcript text and memo fields are excluded. Offline coding is free forever. Hosted collaboration is optional and free during beta. Paid plans and terms for beta users will be announced before charging begins. There is no Fleuron software fee for self-hosting; you are responsible for infrastructure and administration costs.
- **Multi-Format Import**: Native parsing for WebVTT, SRT, plain text with speaker labels, Microsoft Word (`.docx`), and CSV transcripts.
- **Rich Exports**: Export your project to CSV coded passages and framework matrices, standalone HTML reports, and PDF reports.
- **Native Experience**: Native desktop app for macOS 14+ (Universal / Apple Silicon & Intel) and Windows 11 x64. macOS 10.15–13 and Windows 10 are best effort; see the [support matrix](docs/IT-DEPLOYMENT.md).

---

## Getting Started

### Installation

Downloads are distributed from [fleuron.study](https://fleuron.study/) and the [official Releases page](https://github.com/wilsonhyeh/fleuron/releases/latest) (with website downloads pointing directly at files hosted on the GitHub release page). Full step-by-step guidance — including the **expected first-launch warnings** for unsigned-but-official builds and the "stop here" signals — lives in [docs/INSTALLING.md](docs/INSTALLING.md):

> Fleuron is an independent open-source application. This build does not yet carry an Apple Developer ID/notarization or Windows Authenticode publisher signature, so your operating system cannot verify its publisher automatically. Download only from `https://fleuron.study/` or the official release page at `https://github.com/wilsonhyeh/fleuron/releases`. Continue only when the version, filename, and warning match this guide. A malware warning, checksum mismatch, or unexpected administrator request means stop.

| Platform | File |
| --- | --- |
| macOS (Intel + Apple Silicon) | `Fleuron_2.1.0_universal.dmg` |
| Windows 11 x64 | `Fleuron_2.1.0_x64-setup.exe` |

New studies default to a local working library (`~/Fleuron` / `%USERPROFILE%\Fleuron`). Windows installs per-user with no administrator prompt; macOS uses System Settings → Privacy & Security → Open Anyway once.

For detailed usage instructions and workflow tips, see the [User Guide](docs/USER-GUIDE.md), [synthetic transcript-coding tutorial](https://fleuron.study/guides/code-interview-transcripts/) and [reflexive thematic analysis workflow](https://fleuron.study/guides/reflexive-thematic-analysis/).

---

## Quick Workflow

1. **Create or Join a Study**: Start a new study locally (`⌘N` / `Ctrl+N`) or join an existing team study using an 8-character invite code.
2. **Import Transcripts**: Add participant interviews in VTT, SRT, TXT, DOCX, or CSV formats.
3. **Code Passages**: Select text in the transcript panel to bring up the coding bubble. Type an existing code name or enter a new one to create it immediately.
4. **Reflect & Memo**: Record reflexive notes on individual highlights or broader interview-level memos.
5. **Export & Share**: Generate publication-ready codebooks, coded segment listings, and thematic matrices from the **Export** menu (`⇧⌘E` / `Ctrl+Shift+E`).

---

## Architecture & Privacy Model

Fleuron separates **local research data** from **collaborative synchronization metadata**:

```
Project Folder (my-study.fleuron/)
├── project.json       # Project configuration and metadata
├── project.db         # Local SQLite database (transcripts, segments, codes, memos, sync log)
├── interviews/        # Raw source transcripts (stored locally)
└── exports/           # Generated CSV, HTML, and PDF exports
```

- **Local-First Boundary**: Collaboration sync excludes transcript files/text, quote fields, filenames, and memos. Quotes copied into codebook fields sync as written. Exports and backups are saved where you choose and can leave the device through sharing or a synced folder.
- **Sync Protocol v2**: Collaboration syncs account email + study/codebook/coding metadata over encrypted HTTPS/WSS — including codebook definitions, criteria, and examples and your de-identified study/participant labels. Author them de-identified from the start. Automatic update checks contact GitHub at startup and periodically; disable them in Settings if needed. Sign-in and session renewal use credential values with the configured authentication service. The exact field table lives in [docs/PRIVACY-AND-PERMISSIONS.md](docs/PRIVACY-AND-PERMISSIONS.md).
- **No compliance promises**: Fleuron can support a protocol; it does not certify anything.

---

## Development

### Prerequisites

- **Node.js**: 20.x or higher
- **Rust**: Latest stable Rust toolchain via [rustup](https://rustup.rs)
- **Tauri Prerequisites**: See the [Tauri v2 Prerequisites Guide](https://v2.tauri.app/start/prerequisites/) for OS-specific system libraries.

### Setup

```bash
# Clone repository
git clone https://github.com/wilsonhyeh/fleuron.git
cd fleuron

# Install frontend dependencies
npm install

# Run desktop development server (frontend + Tauri backend)
npm run tauri dev

# Run frontend-only browser preview with mock backend
npm run dev
```

### Testing & Verification

```bash
# Run unit and contract tests
npm test

# Run Rust unit tests
cargo test --manifest-path src-tauri/Cargo.toml

# Run Playwright end-to-end tests
npm run test:e2e
```

---

## Methodology

Fleuron is designed to support **reflexive thematic analysis** (Braun & Clarke, 2019, 2021). It emphasizes qualitative rigor, researcher subjectivity, and iterative codebook refinement over inter-rater reliability scores or algorithmic consensus.

---

## Contributing

Contributions, bug reports, and feature suggestions are welcome! Please check [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on development workflows, code standards, and submitting pull requests.

---

## License

Fleuron is open-source software licensed under the [MIT License](LICENSE).
