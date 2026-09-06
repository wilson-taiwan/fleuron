#!/usr/bin/env bash
# Run the complete local-Supabase verification suite for Fleuron.
#
# This wrapper is LOCAL ONLY. It refuses to proceed when Docker or the
# Supabase CLI are unusable, and it never accepts a database URL, never runs
# `supabase link`, `supabase db push`, or any command that could touch a
# non-local project. Every operation targets the local stack via explicit
# `--local` flags or the local default ports.
#
# What it runs, in order:
#   1. supabase start            (idempotent; ignores "already started")
#   2. supabase db reset --local --no-seed
#   3. supabase db lint --local --level error --fail-on error
#   4. supabase test db --local supabase/tests/sync-v2.pgtap.sql
#   5. supabase test db --local supabase/tests/entitlements.pgtap.sql
#   6. bash scripts/verify-supabase-migrations.sh   (static gate)
set -euo pipefail
set +x

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Never run with a production/remote database URL or credentialed identity.
if [[ -n "${SUPABASE_DB_URL:-}" || -n "${SUPABASE_ACCESS_TOKEN:-}" ]]; then
  echo "✗ Refusing to run: SUPABASE_DB_URL / SUPABASE_ACCESS_TOKEN are set." >&2
  echo "  This script is local-only; unset them or run it in a clean shell." >&2
  exit 1
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "✗ Docker CLI is not installed. Install Docker Desktop and start it:"
  echo "    brew install --cask docker"
  exit 1
fi

if ! docker info >/dev/null 2>&1; then
  echo "✗ Docker daemon is not reachable. Start Docker Desktop and wait for"
  echo "  the engine, then rerun: docker info"
  exit 1
fi

if ! command -v supabase >/dev/null 2>&1; then
  echo "✗ Supabase CLI not found. Install it (npm i -g supabase or brew) and"
  echo "  verify with: supabase --version"
  exit 1
fi

cd "$ROOT"

echo "── Supabase CLI: $(supabase --version)"
echo "── Starting local stack …"
supabase start || {
  status=$?
  # `supabase start` exits nonzero when the stack is already running; that is
  # the normal repeat-run case, so confirm rather than fail.
  echo "  (start exited $status — checking whether the stack is healthy)"
  supabase status --local >/dev/null 2>&1 || {
    echo "✗ Local stack could not be started. Inspect: supabase start" >&2
    exit 1
  }
}

echo "── Reset local database (no seed) …"
supabase db reset --local --no-seed

echo "── Lint migrations (error-level, fail on error) …"
supabase db lint --local --level error --fail-on error

echo "── pgTAP: sync-v2 …"
supabase test db --local supabase/tests/sync-v2.pgtap.sql

echo "── pgTAP: entitlements …"
supabase test db --local supabase/tests/entitlements.pgtap.sql

echo "── pgTAP: study-lifecycle …"
supabase test db --local supabase/tests/study-lifecycle.pgtap.sql

echo "── Static migration gate …"
bash scripts/verify-supabase-migrations.sh

echo "All local Supabase gates passed."
