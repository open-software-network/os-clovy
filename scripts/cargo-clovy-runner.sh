#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "run" ]]; then
  exec cargo "$@"
fi

shift

cargo_args=()
app_args=()
after_separator=false
expect_target=false
profile="debug"
target_triple=""

for arg in "$@"; do
  if [[ "$after_separator" == true ]]; then
    app_args+=("$arg")
    continue
  fi

  if [[ "$arg" == "--" ]]; then
    after_separator=true
    continue
  fi

  cargo_args+=("$arg")

  if [[ "$expect_target" == true ]]; then
    target_triple="$arg"
    expect_target=false
    continue
  fi

  case "$arg" in
    --release)
      profile="release"
      ;;
    --target)
      expect_target=true
      ;;
    --target=*)
      target_triple="${arg#--target=}"
      ;;
  esac
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# One code signing identity for the app bundle and every helper build.rs signs:
# an explicit CLOVY_DEV_SIGNING_IDENTITY, or the single valid Apple Development
# identity in the login keychain. Normalized to its SHA-1 before Cargo runs.
CLOVY_DEV_SIGNING_IDENTITY="$(node "$script_dir/dev-signing-identity.mjs")"
export CLOVY_DEV_SIGNING_IDENTITY

# The Computer use helper is prepared and signed separately before Tauri runs.
# Building only Clovy and its MCP relay here
# prevents Cargo from relinking that helper with the app's lower deployment
# target or a different active Swift toolchain. The relay sits
# beside the app executable (docs/mcp-server.md).
cargo build --bin clovy --bin clovy-mcp ${cargo_args[@]+"${cargo_args[@]}"}

target_dir="${CARGO_TARGET_DIR:-target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="$(pwd)/$target_dir"
fi

if [[ -n "$target_triple" ]]; then
  bin_dir="$target_dir/$target_triple/$profile"
else
  bin_dir="$target_dir/$profile"
fi

# Run the app from a signed bundle beside Cargo's output, so macOS can show it in
# System Settings and keep its privacy grants across restarts and rebuilds
# (docs/development.md). The launcher execs the bundle executable in place,
# keeping this process id, the arguments, stdio, and the exit status Tauri dev
# supervises, and makes the app responsible for its own privacy requests.
launch="$(node "$script_dir/dev-app-bundle.mjs" --profile-dir "$bin_dir")"
launcher="${launch%%$'\n'*}"
executable="${launch#*$'\n'}"

if [[ "${#app_args[@]}" -gt 0 ]]; then
  exec "$launcher" "$executable" "${app_args[@]}"
else
  exec "$launcher" "$executable"
fi
