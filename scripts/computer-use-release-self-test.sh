#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
live=false
adhoc=false
target=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --live)
      live=true
      shift
      ;;
    --adhoc)
      adhoc=true
      shift
      ;;
    --target)
      if [[ "$#" -lt 2 || -z "$2" ]]; then
        echo "--target requires a Rust target" >&2
        exit 2
      fi
      target="$2"
      shift 2
      ;;
    --target=*)
      target="${1#--target=}"
      if [[ -z "$target" ]]; then
        echo "--target requires a Rust target" >&2
        exit 2
      fi
      shift
      ;;
    *)
      echo "Usage: $0 [--live] [--adhoc] [--target <rust-target>]" >&2
      exit 2
      ;;
  esac
done

case "$target" in
  ""|universal-apple-darwin|aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *)
    echo "Unsupported Computer use release target: $target" >&2
    exit 2
    ;;
esac

shopt -s nullglob
if [[ -n "$target" ]]; then
  apps=("$ROOT_DIR"/src-tauri/target/"$target"/release/bundle/macos/*.app)
else
  apps=("$ROOT_DIR"/src-tauri/target/release/bundle/macos/*.app)
fi
if [[ "${#apps[@]}" -ne 1 ]]; then
  echo "Expected exactly one signed Clovy app bundle for target ${target:-native}, found ${#apps[@]}." >&2
  exit 1
fi

helper="${apps[0]}/Contents/Resources/native/bin/Clovy Computer Use Driver.app"
host="${apps[0]}/Contents/MacOS/clovy"
args=(--bundle "$helper" --host "$host")
# An ad hoc release (no Apple identity) has no Developer ID to require.
if [[ "$adhoc" == true ]]; then
  args+=(--require-release-profile)
else
  args+=(--require-developer-id)
fi
if [[ "$live" == true ]]; then
  args+=(--live)
fi

cd "$ROOT_DIR"
node scripts/computer-use-self-test.mjs "${args[@]}"
