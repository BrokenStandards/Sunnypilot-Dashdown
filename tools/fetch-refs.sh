#!/usr/bin/env bash
# Reconstruct the gitignored ref/ directory of third-party reference source,
# each repo pinned to an exact commit for reproducibility.
#
# Usage:
#   tools/fetch-refs.sh            # Phase A repos (default)
#   tools/fetch-refs.sh phasea
#   tools/fetch-refs.sh phaseb     # iOS-on-Linux toolchain sources
#   tools/fetch-refs.sh all
#
# ref/ is gitignored (and in .ignore) so it never pollutes git or ripgrep/Grep
# searches over our own code. Search it deliberately with tools/refgrep.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REF_DIR="$REPO_ROOT/ref"
mkdir -p "$REF_DIR"

# pin <dir> <url> <sha>  — shallow, exact-commit, no submodules.
pin() {
  local dir="$1" url="$2" sha="$3"
  local dest="$REF_DIR/$dir"
  if [ -d "$dest/.git" ] && [ "$(git -C "$dest" rev-parse HEAD 2>/dev/null)" = "$sha" ]; then
    echo "ok   $dir already at $sha"
    return
  fi
  echo "pin  $dir -> $sha"
  rm -rf "$dest"
  git init -q "$dest"
  git -C "$dest" remote add origin "$url"
  git -C "$dest" fetch -q --depth 1 origin "$sha"
  git -C "$dest" checkout -q --detach FETCH_HEAD
}

fetch_phasea() {
  pin copyparty      https://github.com/9001/copyparty            6e75faa62349a59f4df328a4939ba8626d89ee1a
  pin sunnypilot     https://github.com/sunnypilot/sunnypilot     46b9253729193e47a8be99154bae41c35359a373
  pin uniffi-rs      https://github.com/mozilla/uniffi-rs         1a6111c32f8be55bfedceddabbf27ec65f4c7755
  pin uniffi-starter https://github.com/ianthetechie/uniffi-starter b466bc276437250cca3b477b4840b49488205a91
}

# Phase B SHAs are filled in when Phase B begins; until then we take branch tips.
fetch_phaseb() {
  pin_tip() { # pin_tip <dir> <url> <branch>
    local dir="$1" url="$2" br="$3" dest="$REF_DIR/$1"
    echo "tip  $dir <- $br (pin a SHA here when Phase B starts)"
    rm -rf "$dest"
    git clone -q --depth 1 --branch "$br" "$url" "$dest"
  }
  pin_tip xtool            https://github.com/xtool-org/xtool                       main
  pin_tip osxcross         https://github.com/tpoechtrager/osxcross                 master
  pin_tip libimobiledevice https://github.com/libimobiledevice/libimobiledevice     master
}

case "${1:-phasea}" in
  phasea) fetch_phasea ;;
  phaseb) fetch_phaseb ;;
  all)    fetch_phasea; fetch_phaseb ;;
  *) echo "usage: $0 [phasea|phaseb|all]" >&2; exit 2 ;;
esac

echo "done. ref/ contents:"
ls -1 "$REF_DIR"
