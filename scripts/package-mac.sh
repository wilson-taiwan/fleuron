#!/usr/bin/env bash
# Build a double-clickable macOS app + DMG for teammate distribution.
#
# History worth knowing (2026-08-15): this script used to locate its artifacts
# with `find … | head -1`, which shipped the wrong files for three releases.
#
#   * `find "$BUNDLE_ROOT/macos" -name '*.app' | head -1` picked whichever app
#     bundle traversal happened to reach first. After the Fleuron rename the
#     directory held both "Qualitative Coding.app" and "Fleuron.app", so
#     releases/Fleuron.app was silently the OLD build.
#   * `find "$BUNDLE_ROOT" -maxdepth 2 -name '*.dmg' | head -1` reached
#     bundle/macos/ before bundle/dmg/ and matched `rw.NNNNN.*.dmg` — the
#     read-write SCRATCH image bundle_dmg.sh leaves behind. Every DMG in
#     releases/ was therefore a 37 MB UDRW volume that does nothing when you
#     double-click it, instead of the 4.7 MB UDZO one next to it.
#
# So: derive the exact expected names, wipe stale bundle output first, and
# assert what was copied is what was meant. No globbing for "a" build artifact.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

export CARGO_TARGET_DIR="$ROOT/src-tauri/target"

VERSION="$(node -p "require('./package.json').version")"
# Product name comes from tauri.conf.json so a rename can never desync it.
PRODUCT_NAME="$(node -p "require('./src-tauri/tauri.conf.json').productName")"
UPDATER_PUBKEY="$(node -p "require('./src-tauri/tauri.conf.json').plugins?.updater?.pubkey ?? ''")"
RELEASE_DIR="$ROOT/releases"
DMG_NAME="${PRODUCT_NAME}-${VERSION}-mac.dmg"
BUNDLE_ROOT="$CARGO_TARGET_DIR/release/bundle"

echo "→ Packaging ${PRODUCT_NAME} ${VERSION}"

if [[ "$UPDATER_PUBKEY" == "REPLACE_WITH_FLEURON_UPDATER_PUBLIC_KEY" || "$UPDATER_PUBKEY" == "__REPLACE_WITH_PUBLIC_UPDATER_KEY__" ]]; then
  echo "Error: replace the updater public key in src-tauri/tauri.conf.json before packaging."
  exit 1
fi
if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "Error: TAURI_SIGNING_PRIVATE_KEY is required for signed updater artifacts."
  exit 1
fi

# Stale bundles from earlier versions (or earlier names) are what made the old
# `find | head -1` ambiguous. Remove the ambiguity rather than ordering around it.
if [[ -d "$BUNDLE_ROOT" ]]; then
  echo "→ Clearing previous bundle output…"
  rm -rf "$BUNDLE_ROOT"
fi

echo "→ Building frontend…"
npm run build

echo "→ Building macOS app bundle (release)…"
npm run tauri build

APP_SRC="$BUNDLE_ROOT/macos/${PRODUCT_NAME}.app"
if [[ ! -d "$APP_SRC" ]]; then
  echo "Error: expected app bundle at $APP_SRC"
  ls -la "$BUNDLE_ROOT/macos" 2>/dev/null || true
  exit 1
fi

UPDATER_SRC="$BUNDLE_ROOT/macos/${PRODUCT_NAME}.app.tar.gz"
UPDATER_SIG_SRC="${UPDATER_SRC}.sig"
if [[ ! -f "$UPDATER_SRC" || ! -f "$UPDATER_SIG_SRC" ]]; then
  echo "Error: updater artifacts are missing. Set TAURI_SIGNING_PRIVATE_KEY and rebuild."
  ls -la "$BUNDLE_ROOT/macos" 2>/dev/null || true
  exit 1
fi

# Only ever look inside bundle/dmg — never bundle/macos, where the scratch
# image lives — and require exactly one match for this name and version.
shopt -s nullglob
DMG_CANDIDATES=("$BUNDLE_ROOT/dmg/${PRODUCT_NAME}_${VERSION}_"*.dmg)
shopt -u nullglob

if [[ ${#DMG_CANDIDATES[@]} -ne 1 ]]; then
  echo "Error: expected exactly 1 DMG matching ${PRODUCT_NAME}_${VERSION}_*.dmg, found ${#DMG_CANDIDATES[@]}"
  ls -la "$BUNDLE_ROOT/dmg" 2>/dev/null || true
  exit 1
fi
DMG_SRC="${DMG_CANDIDATES[0]}"

# A distributable DMG is read-only compressed. UDRW here means we grabbed a
# scratch image again — the exact bug this script shipped three times.
DMG_FORMAT="$(hdiutil imageinfo "$DMG_SRC" 2>/dev/null | awk -F': *' '/^Format:/ {print $2}')"
if [[ "$DMG_FORMAT" != "UDZO" && "$DMG_FORMAT" != "UDBZ" && "$DMG_FORMAT" != "ULFO" ]]; then
  echo "Error: $DMG_SRC has format '$DMG_FORMAT'; expected a read-only compressed image."
  echo "       A UDRW image is bundle_dmg.sh's scratch volume, not a distributable DMG."
  exit 1
fi

mkdir -p "$RELEASE_DIR"
cp -f "$DMG_SRC" "$RELEASE_DIR/$DMG_NAME"
rm -rf "$RELEASE_DIR/${PRODUCT_NAME}.app"
cp -R "$APP_SRC" "$RELEASE_DIR/${PRODUCT_NAME}.app"
cp -f "$UPDATER_SRC" "$RELEASE_DIR/${PRODUCT_NAME}-${VERSION}-mac.app.tar.gz"
cp -f "$UPDATER_SIG_SRC" "$RELEASE_DIR/${PRODUCT_NAME}-${VERSION}-mac.app.tar.gz.sig"

# Verify the copied bundle.
bash "$ROOT/scripts/verify-mac-bundle.sh" \
  "$RELEASE_DIR/${PRODUCT_NAME}.app" "$VERSION" "$ROOT/dist"

COPIED_NAME="$PRODUCT_NAME"
COPIED_VERSION="$VERSION"

cat <<EOF

✓ Candidate build ready — verified $COPIED_NAME $COPIED_VERSION, DMG format $DMG_FORMAT

  DMG:          releases/$DMG_NAME  ($(du -h "$RELEASE_DIR/$DMG_NAME" | cut -f1))
  App:          releases/${PRODUCT_NAME}.app
  Updater:      releases/${PRODUCT_NAME}-${VERSION}-mac.app.tar.gz[.sig]

Canonical distribution is the GitHub Releases page:
  https://github.com/wilsonhyeh/fleuron/releases
Upload this artifact through a candidate workflow rather than sharing files
by hand — the release page, exact filename, and version are part of how users
verify they have the official download.

Install flow (public docs: docs/INSTALLING.md):
  1. Download $DMG_NAME from the official Releases page
  2. Double-click the DMG and drag "$PRODUCT_NAME" into Applications
  3. Launch; macOS shows the expected non-notarized warning

First launch warning (the ONLY supported path):
  System Settings → Privacy & Security → Open Anyway → Open.
  A "damaged" or malware warning means stop — see the install guide.

EOF
