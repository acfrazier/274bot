#!/usr/bin/env bash
# Local convenience runner only; CI does not use this script. CI inlines the
# clone plus parser-pin plus check.py steps in .github/workflows/ci.yml and
# never executes a candidate-tree script. Candidate trees supply source data,
# not rules.
set -euo pipefail

product_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
root="$product_root"
if [[ $# -ne 0 ]]; then
    if [[ $# -ne 2 || $1 != --root ]]; then
        echo "usage: $0 [--root <tree>]" >&2
        exit 2
    fi
    root="$2"
fi
root="$(cd -- "$root" && pwd)"
arch_repo="${ARCH_REPO:-https://github.com/acfrazier/274bot-arch.git}"
cache_dir="${ARCH_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/274bot-arch}"
source_ref=refs/heads/main
# Explicit local overrides exercise prepared commits before the operator pushes.
if [[ -d "$arch_repo" ]]; then
    arch_repo="$(cd -- "$arch_repo" && pwd)"
    source_ref=HEAD
fi

parser="$(command -v rust-analyzer)" || {
    echo "architecture: pinned rust-analyzer is not on PATH" >&2
    exit 2
}
platform="$(uname -s)/$(uname -m)"
case "$platform" in
    Darwin/arm64)
        parser_sha256=c863e967a7244fa3687d7bd6235ac7abae5a7ce44bcbf8307eb1dc80aa85f397
        ;;
    Linux/x86_64)
        parser_sha256=fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49
        ;;
    *)
        echo "architecture: no pinned rust-analyzer SHA-256 for $platform" >&2
        exit 2
        ;;
esac
checksum="$parser_sha256  $parser"
checksum_probe='e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  /dev/null'
if command -v sha256sum >/dev/null 2>&1 &&
    printf '%s\n' "$checksum_probe" | sha256sum --check --status >/dev/null 2>&1; then
    printf '%s\n' "$checksum" | sha256sum --check --status || {
        echo "architecture: rust-analyzer does not match the pinned SHA-256" >&2
        exit 2
    }
elif command -v shasum >/dev/null 2>&1 &&
    printf '%s\n' "$checksum_probe" | shasum -a 256 -c -s >/dev/null 2>&1; then
    printf '%s\n' "$checksum" | shasum -a 256 -c -s || {
        echo "architecture: rust-analyzer does not match the pinned SHA-256" >&2
        exit 2
    }
else
    echo "architecture: no supported SHA-256 checksum utility is available" >&2
    exit 2
fi
parser_version="$("$parser" --version)"
case "$parser_version" in
    'rust-analyzer 0.3.3081-standalone'|'rust-analyzer 0.3.3081-standalone ('*) ;;
    *)
        echo "architecture: rust-analyzer does not match the pinned version" >&2
        exit 2
        ;;
esac

if [[ ! -d "$cache_dir/objects" ]]; then
    git init --bare --quiet "$cache_dir"
fi
git --git-dir="$cache_dir" fetch --quiet --depth=1 "$arch_repo" "$source_ref"
checker_root="$(mktemp -d "${TMPDIR:-/tmp}/274bot-arch.XXXXXXXX")"
trap 'rm -rf -- "$checker_root"' EXIT
git --git-dir="$cache_dir" archive FETCH_HEAD | tar -x -C "$checker_root"
checker_python=''
for minor in {99..11}; do
    candidate="$(command -v "python3.$minor" || true)"
    if [[ -n "$candidate" ]] &&
        "$candidate" -c 'import sys; raise SystemExit(sys.version_info < (3, 11))' >/dev/null 2>&1; then
        checker_python="$candidate"
        break
    fi
done
if [[ -z "$checker_python" ]]; then
    candidate="$(command -v python3 || true)"
    if [[ -n "$candidate" ]] &&
        "$candidate" -c 'import sys; raise SystemExit(sys.version_info < (3, 11))' >/dev/null 2>&1; then
        checker_python="$candidate"
    fi
fi
if [[ -z "$checker_python" ]]; then
    echo "architecture: Python 3.11 or newer is not on PATH" >&2
    exit 2
fi
"$checker_python" -B "$checker_root/check.py" --self-test
"$checker_python" -B "$checker_root/check.py" --root "$root"
