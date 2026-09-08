#!/usr/bin/env bash
# Read-only verifier for linux-failure-attribution archive identity.
# Usage: from campaign checkout root, or pass ARCHIVE_ROOT.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ARCH_ROOT="${1:-$ROOT/diagnostics/linux-failure-attribution-20260908}"
TAR="$ARCH_ROOT/archive-d4a3ed5.tar.gz"
TREE="$ARCH_ROOT/archive-d4a3ed5"
MANIFEST="$TREE/archive-manifest.json"
EXPECT_TAR_SHA="40444f4b6af3ad6bb82b13384dd7ed52e92485e09b6ece3ac3ed247d2efb2750"
EXPECT_FILES=25

if [[ ! -f "$TAR" || ! -f "$MANIFEST" ]]; then
  echo "missing tar or manifest under $ARCH_ROOT" >&2
  exit 2
fi

got_tar="$(shasum -a 256 "$TAR" | awk '{print $1}')"
if [[ "$got_tar" != "$EXPECT_TAR_SHA" ]]; then
  echo "tar sha mismatch: got $got_tar want $EXPECT_TAR_SHA" >&2
  exit 1
fi
echo "tar ok $got_tar"

n="$(jq '.files|length' "$MANIFEST")"
if [[ "$n" != "$EXPECT_FILES" ]]; then
  echo "manifest file count $n != $EXPECT_FILES" >&2
  exit 1
fi

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
jq -r '.files[] | "\(.sha256)  '"$TREE"'/\(.path)"' "$MANIFEST" >"$tmp"
shasum -a 256 -c "$tmp"
echo "manifest $n files ok"

# Spot-check failure boundary attribution shape (no reinterpretation).
qual="$TREE/raw-run-01/samples.qualification.jsonl"
attr="$(jq -c 'select(.record=="failure-boundary") | .slots[] | select(.error!=null) | .runtime.failure_attribution' "$qual")"
echo "failure_attribution=$attr"
echo "$attr" | jq -e '
  .tick==19
  and .call_path=="sync"
  and .error_variant=="Runtime"
  and .terminating_before_cancel==true
  and .interrupt_id==1
' >/dev/null
echo "attribution spot-check ok"
echo "status remains failed / not qualified (verifier does not change artifacts)"
