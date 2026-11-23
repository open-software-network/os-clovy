# Local command layer mirroring desktop CI.
# Use `make dev` for the native app and Vite; providers are called directly.
.PHONY: help install \
	dev \
	check format typecheck test-web \
	computer-use-prepare tauri-fmt tauri-fmt-check tauri-lint tauri-test \
	companion-fmt companion-fmt-check companion-lint companion-test \
	fmt fmt-check lint test verify org-references-check clovy-api-references-check \
	local-ci signoff-pr signoff-frontend signoff-rust-macos

.DEFAULT_GOAL := help

help:  ## Show this help
	@grep -hE '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| sort \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'

# --- Install ---
install:  ## Install frontend deps (Rust builds via cargo)
	sfw pnpm install --frozen-lockfile

# --- Run (local dev) ---
# `pnpm tauri:dev` selects a free worktree-local Vite port.
dev:  ## Run the desktop app and Vite (Ctrl-C stops both)
	pnpm tauri:dev

# --- Frontend (src/, scripts/) ---
check:  ## Biome check (format + lint, incl. the lucide ban)
	pnpm check

format:  ## Biome format (write) + biome safe fixes
	pnpm check:write

typecheck:  ## tsc --noEmit
	pnpm typecheck

test-web:  ## Vitest
	pnpm test

# --- Tauri shell (src-tauri/) ---
tauri-fmt:  ## rustfmt (write)
	cargo fmt --manifest-path src-tauri/Cargo.toml --all

tauri-fmt-check:  ## rustfmt (check only)
	cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check

computer-use-prepare:  ## Prepare the real pinned Computer use engine and helper
	node scripts/prepare-computer-use.mjs

tauri-lint: computer-use-prepare  ## clippy (warnings = errors)
	cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings

tauri-test: computer-use-prepare  ## cargo test
	cargo test --manifest-path src-tauri/Cargo.toml --locked

# --- Companion protocol and crypto crates (crates/clovy-companion-*) ---
companion-fmt:  ## rustfmt companion crates (write)
	cargo fmt --manifest-path crates/clovy-companion-protocol/Cargo.toml --all
	cargo fmt --manifest-path crates/clovy-companion-crypto/Cargo.toml --all
	cargo fmt --manifest-path crates/june-companion-protocol/Cargo.toml --all
	cargo fmt --manifest-path crates/june-companion-crypto/Cargo.toml --all

companion-fmt-check:  ## rustfmt companion crates (check only)
	cargo fmt --manifest-path crates/clovy-companion-protocol/Cargo.toml --all -- --check
	cargo fmt --manifest-path crates/clovy-companion-crypto/Cargo.toml --all -- --check
	cargo fmt --manifest-path crates/june-companion-protocol/Cargo.toml --all -- --check
	cargo fmt --manifest-path crates/june-companion-crypto/Cargo.toml --all -- --check

companion-lint:  ## clippy companion crates (warnings = errors)
	cargo clippy --manifest-path crates/clovy-companion-protocol/Cargo.toml --all-targets --locked -- -D warnings
	cargo clippy --manifest-path crates/clovy-companion-crypto/Cargo.toml --all-targets --locked -- -D warnings
	cargo clippy --manifest-path crates/june-companion-protocol/Cargo.toml --all-targets --locked -- -D warnings
	cargo clippy --manifest-path crates/june-companion-crypto/Cargo.toml --all-targets --locked -- -D warnings

companion-test:  ## cargo test companion crates
	cargo test --manifest-path crates/clovy-companion-protocol/Cargo.toml --locked
	cargo test --manifest-path crates/clovy-companion-crypto/Cargo.toml --locked
	cargo test --manifest-path crates/june-companion-protocol/Cargo.toml --locked
	cargo test --manifest-path crates/june-companion-crypto/Cargo.toml --locked

.PHONY: benchmark-note-transcription-latency benchmark-calendar-account-poll
benchmark-note-transcription-latency:
	cargo test --manifest-path src-tauri/Cargo.toml --locked --release commands::note_transcription_benchmark::benchmark_post_finalization_note_transcription_latency -- --ignored --exact --nocapture --test-threads=1

benchmark-calendar-account-poll:
	cargo test --manifest-path src-tauri/Cargo.toml --locked --release connectors::triggers::tests::benchmark_calendar_account_poll_consolidation -- --ignored --exact --nocapture --test-threads=1

# --- Aggregates ---
fmt: format tauri-fmt companion-fmt  ## Format everything

fmt-check: tauri-fmt-check companion-fmt-check  ## Check Rust formatting

lint: check tauri-lint companion-lint  ## Lint everything

test: test-web tauri-test companion-test  ## Run all test suites

org-references-check:  ## Fail on Open Software organization references and the June-era app identity
	node scripts/check-org-references.mjs

clovy-api-references-check:  ## Reject retired server identifiers and routes
	node scripts/check-clovy-api-references.mjs

verify: clovy-api-references-check org-references-check check typecheck test-web tauri-fmt-check tauri-lint tauri-test companion-fmt-check companion-lint companion-test  ## Full CI-parity gate

local-ci:  ## Run path-aware local PR checks and post required signoff/* statuses
	./scripts/local-ci.sh

signoff-pr: local-ci

signoff-frontend:  ## Run local frontend typecheck/tests and post signoff/frontend
	./scripts/signoff-frontend.sh

signoff-rust-macos:  ## Run local macOS Tauri Rust checks and post signoff/rust-macos
	./scripts/signoff-rust-macos.sh
