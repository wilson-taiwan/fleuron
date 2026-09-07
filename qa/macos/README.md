# Fleuron macOS QA Runner

This runner verifies the matching Fleuron DMG without installing the app or bypassing Gatekeeper. It uses macOS-built-in `hdiutil`, `codesign`, and `PlistBuddy` to verify the disk image, mounted bundle, app version, bundle signature, and packaged-app `--selftest`.

## Release asset

**This runner is a maintainer tool and is deliberately not published as a release asset.** To verify a specific release, check out that release's tag and use the `qa/macos/` directory from that commit:

```
https://github.com/wilsonhyeh/fleuron/archive/refs/tags/v<version>.zip
```

A `release.json` beside `Invoke-FleuronQA.sh` still pins the expected release version and rejects a canonical DMG filename for a different release, but nothing generates one any more.

## Run

1. Unzip the macOS QA runner archive.
2. Download the matching `Fleuron_<version>_universal.dmg` from the same GitHub Release.
3. Run:

   ```bash
   bash ./Fleuron-macOS-QA-Runner/Invoke-FleuronQA.sh \
     --candidate ./Fleuron_<version>_universal.dmg
   ```

The runner writes `fleuron-qa-evidence/SUMMARY.md` and raw command logs in the current directory. It mounts the DMG read-only, runs the packaged app's self-test, then detaches the DMG. It never copies the app to Applications, changes quarantine flags, opens Security & Privacy settings, or advises bypassing any OS security control.

When used from a source checkout instead of a release asset, provide a canonical DMG filename or pass `--expected-version X.Y.Z`.
