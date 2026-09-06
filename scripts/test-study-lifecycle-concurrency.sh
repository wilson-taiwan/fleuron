#!/usr/bin/env bash
# Local-only concurrency test runner for study lifecycle operations.
# Verifies that concurrent leave_group operations cannot leave a project memberless.
set -euo pipefail
set +x

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ -n "${SUPABASE_DB_URL:-}" || -n "${SUPABASE_ACCESS_TOKEN:-}" ]]; then
  echo "✗ Refusing to run: SUPABASE_DB_URL / SUPABASE_ACCESS_TOKEN are set." >&2
  exit 1
fi

if ! command -v supabase >/dev/null 2>&1; then
  echo "✗ Supabase CLI not found." >&2
  exit 1
fi

cd "$ROOT"

echo "── Running study lifecycle pgTAP test suite …"
supabase test db --local supabase/tests/study-lifecycle.pgtap.sql
echo "✓ Study lifecycle pgTAP tests passed."
