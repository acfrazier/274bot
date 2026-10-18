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
printf '%s  %s\n' \
    fd89304c85f6c1996b036ea8f2c9ad9d3e25a197dbd713661c7fe51ec9dc8e49 \
    "$parser" | sha256sum --check --status || {
    echo "architecture: rust-analyzer does not match the pinned SHA-256" >&2
    exit 2
}
if [[ "$("$parser" --version)" != 'rust-analyzer 0.3.3081-standalone' ]]; then
    echo "architecture: rust-analyzer does not match the pinned version" >&2
    exit 2
fi

if [[ ! -d "$cache_dir/objects" ]]; then
    git init --bare --quiet "$cache_dir"
fi
git --git-dir="$cache_dir" fetch --quiet --depth=1 "$arch_repo" "$source_ref"
checker_root="$(mktemp -d "${TMPDIR:-/tmp}/274bot-arch.XXXXXXXX")"
trap 'rm -rf -- "$checker_root"' EXIT
git --git-dir="$cache_dir" archive FETCH_HEAD | tar -x -C "$checker_root"
python3 -B "$checker_root/check.py" --self-test
python3 -B "$checker_root/check.py" --root "$root"
