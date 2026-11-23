#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SIGNING_ENV_FILE="${SIGNING_ENV_FILE:-$ROOT_DIR/.env.signing}"
TEMP_DIR=""
CERTIFICATE_FILE=""

cleanup() {
  if [[ -n "$TEMP_DIR" && -d "$TEMP_DIR" ]]; then
    rm -rf "$TEMP_DIR"
  fi
}

trap cleanup EXIT INT TERM

check_only=0
if [[ "${1:-}" == "--check" ]]; then
  check_only=1
  shift
fi

ensure_temp_dir() {
  if [[ -z "$TEMP_DIR" ]]; then
    TEMP_DIR="$(mktemp -d)"
  fi
}

import_signing_certificate() {
  ensure_temp_dir
  local keychain="$TEMP_DIR/clovy-signing.keychain-db"
  local keychain_password="clovy-$RANDOM-$$"

  security create-keychain -p "$keychain_password" "$keychain" >/dev/null
  security set-keychain-settings -lut 21600 "$keychain" >/dev/null
  security unlock-keychain -p "$keychain_password" "$keychain" >/dev/null
  security import "$CERTIFICATE_FILE" \
    -k "$keychain" \
    -P "$APPLE_CERTIFICATE_PASSWORD" \
    -T /usr/bin/codesign \
    -T /usr/bin/security \
    >/dev/null
  security set-key-partition-list \
    -S apple-tool:,apple:,codesign: \
    -s \
    -k "$keychain_password" \
    "$keychain" \
    >/dev/null
  security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"') >/dev/null
}

if [[ -f "$SIGNING_ENV_FILE" ]]; then
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line#"${line%%[![:space:]]*}"}"
    [[ -z "$line" || "$line" == \#* ]] && continue
    if [[ "$line" != *=* ]]; then
      echo "Invalid signing env line in $SIGNING_ENV_FILE: $line" >&2
      exit 1
    fi
    key="${line%%=*}"
    value="${line#*=}"
    key="${key%"${key##*[![:space:]]}"}"
    if [[ ! "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
      echo "Invalid signing env key in $SIGNING_ENV_FILE: $key" >&2
      exit 1
    fi
    if [[ ${#value} -ge 2 ]]; then
      first="${value:0:1}"
      last="${value: -1}"
      if [[ ("$first" == "\"" && "$last" == "\"") || ("$first" == "'" && "$last" == "'") ]]; then
        value="${value:1:${#value}-2}"
      fi
    fi
    export "$key=$value"
  done < "$SIGNING_ENV_FILE"
fi

# CI passes an absent secret as an empty variable, and Tauri tries to import a
# certificate as soon as APPLE_CERTIFICATE exists: an empty value is no value.
apple_variables=(
  APPLE_SIGNING_IDENTITY APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_CERTIFICATE_PATH
  APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH APPLE_API_KEY_P8 APPLE_API_KEY_P8_BASE64
)
for name in "${apple_variables[@]}"; do
  [[ -n "${!name:-}" ]] || unset "$name"
done

# What the variables present select. Without APPLE_SIGNING_IDENTITY the app
# and its helpers are signed ad hoc and nothing is notarized; with it they are
# signed by that identity; with the identity and an App Store Connect key the
# DMG is also notarized and stapled. TAURI_SIGNING_PRIVATE_KEY turns on the
# signed updater package (src-tauri/tauri.release.conf.json).
signing="adhoc"
notarize="no"
updater="off"
identity_value="${APPLE_SIGNING_IDENTITY:-}"
[[ -n "${identity_value// /}" ]] && signing="identity"
[[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]] && updater="on"

notary_key_source=""
if [[ -n "${APPLE_API_KEY_PATH:-}${APPLE_API_KEY_P8:-}${APPLE_API_KEY_P8_BASE64:-}" ]]; then
  notary_key_source="set"
fi
if [[ -n "${APPLE_API_ISSUER:-}${APPLE_API_KEY:-}$notary_key_source" ]]; then
  if [[ "$signing" != "identity" ]]; then
    echo "Notarization settings need APPLE_SIGNING_IDENTITY: an ad hoc build cannot be notarized." >&2
    exit 1
  fi
  missing=0
  for name in APPLE_API_ISSUER APPLE_API_KEY; do
    if [[ -z "${!name:-}" ]]; then
      echo "${name} is required to notarize the DMG." >&2
      missing=1
    fi
  done
  if [[ -z "$notary_key_source" ]]; then
    echo "One of APPLE_API_KEY_PATH, APPLE_API_KEY_P8, or APPLE_API_KEY_P8_BASE64 is required to notarize the DMG." >&2
    missing=1
  fi
  [[ "$missing" -eq 0 ]] || exit 1
  notarize="yes"
fi

# An ad hoc build signs nothing with a certificate: Tauri must not see one.
if [[ "$signing" == "adhoc" ]]; then
  unset APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_CERTIFICATE_PATH
fi

if [[ "$check_only" == "1" ]]; then
  tauri_apple_env="$(env | awk -F= '/^APPLE_/ {print $1}' | sort | paste -sd, -)"
  printf 'signing=%s\nnotarize=%s\nupdater=%s\ntauri_apple_env=%s\n' \
    "$signing" "$notarize" "$updater" "$tauri_apple_env"
  exit 0
fi

# A certificate in the environment (CI) goes into a throwaway keychain; a
# local identity is read from the login keychain as it is.
if [[ "$signing" == "identity" && -n "${APPLE_CERTIFICATE:-}${APPLE_CERTIFICATE_PATH:-}" ]]; then
  if [[ -z "${APPLE_CERTIFICATE_PASSWORD:-}" ]]; then
    echo "APPLE_CERTIFICATE_PASSWORD is required to import the signing certificate." >&2
    exit 1
  fi
  if [[ -n "${APPLE_CERTIFICATE_PATH:-}" ]]; then
    if [[ "$APPLE_CERTIFICATE_PATH" != /* ]]; then
      APPLE_CERTIFICATE_PATH="$ROOT_DIR/$APPLE_CERTIFICATE_PATH"
    fi
    if [[ ! -f "$APPLE_CERTIFICATE_PATH" ]]; then
      echo "APPLE_CERTIFICATE_PATH does not point to a readable file: $APPLE_CERTIFICATE_PATH" >&2
      exit 1
    fi
    CERTIFICATE_FILE="$APPLE_CERTIFICATE_PATH"
  else
    if ! printf '%s' "$APPLE_CERTIFICATE" | base64 --decode >/dev/null 2>&1; then
      echo "APPLE_CERTIFICATE is not valid one-line base64. Regenerate it with: base64 -i /path/to/certificate.p12 | tr -d '\\n'" >&2
      exit 1
    fi
    ensure_temp_dir
    CERTIFICATE_FILE="$TEMP_DIR/apple-certificate.p12"
    printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$CERTIFICATE_FILE"
  fi
  import_signing_certificate
fi

if [[ "$notarize" == "yes" ]]; then
  if [[ -n "${APPLE_API_KEY_PATH:-}" ]]; then
    if [[ "$APPLE_API_KEY_PATH" != /* ]]; then
      export APPLE_API_KEY_PATH="$ROOT_DIR/$APPLE_API_KEY_PATH"
    fi
    if [[ ! -f "$APPLE_API_KEY_PATH" ]]; then
      echo "APPLE_API_KEY_PATH does not point to a readable file: $APPLE_API_KEY_PATH" >&2
      exit 1
    fi
  elif [[ -n "${APPLE_API_KEY_P8_BASE64:-}" ]]; then
    ensure_temp_dir
    export APPLE_API_KEY_PATH="$TEMP_DIR/AuthKey_${APPLE_API_KEY}.p8"
    printf '%s' "$APPLE_API_KEY_P8_BASE64" | base64 --decode > "$APPLE_API_KEY_PATH"
  else
    ensure_temp_dir
    export APPLE_API_KEY_PATH="$TEMP_DIR/AuthKey_${APPLE_API_KEY}.p8"
    printf '%s' "$APPLE_API_KEY_P8" | perl -pe 's/\\n/\n/g' > "$APPLE_API_KEY_PATH"
  fi
else
  # Tauri notarizes on its own whenever these are set; an ad hoc or
  # identity-only build must not try.
  unset APPLE_API_ISSUER APPLE_API_KEY APPLE_API_KEY_PATH
fi

# Xcode 27 corrupts proc-macro dylibs when Cargo strips release build
# dependencies under MACOSX_DEPLOYMENT_TARGET=14.0; the shipped binary is
# unaffected by keeping them unstripped.
export CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none

cd "$ROOT_DIR"
# Build the nested helper for the same universal target as the Tauri app before
# the generic before-build hook runs. The preparation stamp lets that hook reuse
# the universal release helper instead of replacing it with the runner's slice.
computer_use_target="universal-apple-darwin"
computer_use_prepare_args=(--release --target "$computer_use_target")
tauri_build_args=()
build_args=("$@")
for ((index = 0; index < ${#build_args[@]}; index += 1)); do
  argument="${build_args[$index]}"
  if [[ "$argument" == "--target" ]]; then
    if ((index + 1 >= ${#build_args[@]})); then
      echo "--target requires a value" >&2
      exit 2
    fi
    requested_target="${build_args[$((index + 1))]}"
    if [[ "$requested_target" != "$computer_use_target" ]]; then
      echo "macOS releases require --target $computer_use_target, got $requested_target." >&2
      exit 2
    fi
    ((index += 1))
    continue
  fi
  if [[ "$argument" == --target=* ]]; then
    requested_target="${argument#--target=}"
    if [[ "$requested_target" != "$computer_use_target" ]]; then
      echo "macOS releases require --target $computer_use_target, got $requested_target." >&2
      exit 2
    fi
    continue
  fi
  tauri_build_args+=("$argument")
done
if [[ "$signing" == "adhoc" ]]; then
  tauri_build_args+=(--config '{"bundle":{"macOS":{"signingIdentity":"-"}}}')
fi
if [[ "$updater" == "on" ]]; then
  tauri_build_args+=(--config src-tauri/tauri.release.conf.json)
fi
# Cargo validates every bundled helper resource before the Tauri build hook.
pnpm browser:prepare -- --release --target "$computer_use_target"
pnpm local-transcriber:prepare -- --release
pnpm computer-use:prepare -- "${computer_use_prepare_args[@]}"
# Trailing args after `--` reach the cargo runner; --locked keeps the
# release build from re-resolving past Cargo.lock (spec/package-install-security.md).
pnpm tauri build --bundles app,dmg --target "$computer_use_target" "${tauri_build_args[@]}" -- --locked

# Validate the copy inside the signed app, not the pre-bundle staging resource.
# Hosted staging runners run the deterministic contract gate; a pre-granted
# desktop release runner can opt into the live TCC/capture/background-action
# fixture with CLOVY_COMPUTER_USE_LIVE_SELF_TEST=1.
computer_use_self_test_args=()
if [[ -n "$computer_use_target" ]]; then
  computer_use_self_test_args+=(--target "$computer_use_target")
fi
if [[ "${CLOVY_COMPUTER_USE_LIVE_SELF_TEST:-0}" == "1" ]]; then
  computer_use_self_test_args+=(--live)
fi
if [[ "$signing" == "adhoc" ]]; then
  computer_use_self_test_args+=(--adhoc)
fi
./scripts/computer-use-release-self-test.sh "${computer_use_self_test_args[@]}"

shopt -s nullglob
apps=(
  "$ROOT_DIR"/src-tauri/target/universal-apple-darwin/release/bundle/macos/*.app
  "$ROOT_DIR"/src-tauri/target/release/bundle/macos/*.app
)
if [[ "${#apps[@]}" -ne 1 ]]; then
  echo "Expected exactly one universal app bundle after build, found ${#apps[@]}." >&2
  exit 1
fi
app="${apps[0]}"
node scripts/prepare-agent-browser.mjs --release --check --target "$computer_use_target" \
  --resource-root "$app/Contents/Resources/native/agent-browser"
if find "$app/Contents/Resources" \( -iname '*hermes*' -o -iname 'python.exe' -o -iname 'python3' \) -print -quit | grep -q .; then
  echo "The app still contains a Hermes or Python payload." >&2
  exit 1
fi
codesign --verify --deep --strict --verbose=2 "$app"
node scripts/verify-macos-app-name.mjs "$app"

dmgs=(
  "$ROOT_DIR"/src-tauri/target/*-apple-darwin/release/bundle/dmg/*.dmg
  "$ROOT_DIR"/src-tauri/target/release/bundle/dmg/*.dmg
)

if [[ "${#dmgs[@]}" -ne 1 ]]; then
  echo "Expected exactly one DMG after build, found ${#dmgs[@]}." >&2
  exit 1
fi
dmg="${dmgs[0]}"

if [[ "$notarize" == "yes" ]]; then
  xcrun notarytool submit "$dmg" \
    --key "$APPLE_API_KEY_PATH" \
    --key-id "$APPLE_API_KEY" \
    --issuer "$APPLE_API_ISSUER" \
    --wait
  xcrun stapler staple "$dmg"
  xcrun stapler validate "$dmg"
  spctl --assess --type install --verbose "$dmg"
fi

# Stable names, so the latest release always serves the same file names.
out_dir="${CLOVY_RELEASE_OUT_DIR:-$ROOT_DIR/src-tauri/target/release-artifacts/macos}"
rm -rf "$out_dir"
mkdir -p "$out_dir"
cp "$dmg" "$out_dir/Clovy_universal.dmg"
if [[ "$updater" == "on" ]]; then
  bundle_dir="$(dirname "$app")"
  [[ -s "$bundle_dir/Clovy.app.tar.gz" && -s "$bundle_dir/Clovy.app.tar.gz.sig" ]] || {
    echo "The updater package or its signature is missing next to $app." >&2
    exit 1
  }
  cp "$bundle_dir/Clovy.app.tar.gz" "$bundle_dir/Clovy.app.tar.gz.sig" "$out_dir/"
fi
echo "macOS release ($signing, notarized: $notarize, updater: $updater) in $out_dir:"
ls -1 "$out_dir"
