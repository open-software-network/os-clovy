#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

if ! command -v gh >/dev/null 2>&1; then
  echo "GitHub CLI is required. Install it from https://cli.github.com/." >&2
  exit 1
fi

if ! gh auth status -h github.com >/dev/null 2>&1; then
  echo "GitHub CLI is not authenticated. Run: gh auth login" >&2
  exit 1
fi

if ! gh extension list | grep -Eq '(^|[[:space:]])basecamp/gh-signoff([[:space:]]|$)'; then
  echo "gh-signoff is required. Run: gh extension install basecamp/gh-signoff" >&2
  exit 1
fi

if ! git rev-parse --abbrev-ref '@{push}' >/dev/null 2>&1; then
  echo "The current branch is not tracking a remote branch. Push it first:" >&2
  echo "  git push -u origin HEAD" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Commit or stash local changes before signing off. The signoff is tied to HEAD." >&2
  exit 1
fi

if [[ -n "$(git log '@{push}'..)" ]]; then
  echo "Push the current HEAD before signing off. The signoff status is posted to the pushed commit." >&2
  echo "  git push" >&2
  exit 1
fi

base_ref="$(gh pr view --json baseRefName --jq .baseRefName 2>/dev/null || true)"
base_ref="${base_ref:-main}"
git fetch origin "$base_ref" --quiet
base_sha="$(git merge-base HEAD "origin/$base_ref")"
changed_files="$(git diff --name-only "$base_sha"...HEAD)"

needs_frontend=false
needs_rust_macos=false
needs_companion=false

if printf '%s\n' "$changed_files" | grep -Eq '^(\.github/actions/setup-pnpm/|\.github/workflows/desktop\.yml$|biome\.json$|hud\.html$|index\.html$|meeting-hud\.html$|package\.json$|pnpm-lock\.yaml$|pnpm-workspace\.yaml$|public/|scripts/|src/|tsconfig\.json$|vite\.config\.ts$)'; then
  needs_frontend=true
fi

if printf '%s\n' "$changed_files" | grep -Eq '^(crates/clovy-companion-(crypto|protocol)/|src-tauri/|\.github/workflows/desktop\.yml$)'; then
  needs_rust_macos=true
fi

if printf '%s\n' "$changed_files" | grep -Eq '^(crates/clovy-companion-(crypto|protocol)/|\.github/workflows/desktop\.yml$|Makefile$|scripts/local-ci\.sh$)'; then
  needs_companion=true
fi

if [[ "$needs_companion" == "true" ]]; then
  make companion-fmt-check
  make companion-lint
  make companion-test
fi

if [[ "$needs_frontend" == "true" ]]; then
  make typecheck
  make test-web
else
  echo "No frontend-signoff paths changed; posting signoff/frontend as not applicable."
fi
gh signoff frontend

if [[ "$needs_rust_macos" == "true" ]]; then
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "signoff/rust-macos is required for this diff and must run from macOS." >&2
    exit 1
  fi
  make tauri-fmt-check
  make tauri-lint
  make tauri-test
else
  echo "No macOS Rust signoff paths changed; posting signoff/rust-macos as not applicable."
fi
gh signoff rust-macos
