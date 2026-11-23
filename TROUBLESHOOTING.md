# Troubleshooting

Failures that look like a bug in the repo and are not. Read this before
"fixing" a red gate, and add to it when a false alarm costs you an hour.

The rule of thumb: **a check that fails locally and passes in CI is usually
your machine, and a check that passes locally and fails in CI is usually you.**
Both halves matter; neither is an excuse to skip the failure.

## Frontend tests

### Storage tests fail locally: `font-scale`

Symptom: `src/test/font-scale.test.ts` fails (storage related) on your machine
while `main` is green in CI.

Cause: **Node 26 ships an experimental web-storage implementation and it
shadows jsdom's `localStorage`.** The tests mock jsdom's; Node's real one wins.
Nothing in the repo is broken.

Fix:

```bash
NODE_OPTIONS=--no-experimental-webstorage pnpm test
```

Check with `node --version`. CI pins an older Node, which is why it does not
see this. Do not "fix" the tests: you would be breaking them for CI to satisfy
your local Node.

### Vitest exits non-zero with zero real failures

`hud-meeting.test.ts` can produce teardown noise that sets a non-zero exit code
even when nothing failed. **Judge vitest by the failure count, not the exit
code.** Read the `Tests  N failed | M passed` line.

### `localsInner` crash in composer / ProseMirror tests

A `@tiptap/pm` duplicate-instance flake that shows up under machine load (for
example while other builds are running). Re-run the file on its own before
believing it:

```bash
pnpm vitest run src/test/agent-workspace.test.tsx -t "<the test name>"
```

If it passes in isolation, it is the flake, not a regression.

## Rust

### `cargo test --lib` says everything passes, CI still fails

`cargo test --lib` runs **only** the unit tests inside the crate. The
integration suites in `src-tauri/tests/*.rs` are separate test binaries; they
are CI-gated and invisible to `--lib`. Run `pnpm test:rust` (or `make
tauri-test`) before believing a Rust change is green.

### Narrow gates pass, `make verify` fails

`make verify` is the CI-parity gate and adds `cargo clippy --all-targets`,
which the narrower targets skip. Clippy failures are hard errors in CI. A
delegate reporting "all green" from `cargo test` + `pnpm test` has **not** run
what CI runs.

Always finish with:

```bash
make verify
```

### Sandbox / permission test failures in a restricted runner

A couple of macOS tests exercise the real Seatbelt sandbox and need a writable
temp dir and permission to spawn `sandbox-exec`. Inside a restricted agent
sandbox they fail for environmental reasons. `TMPDIR=/private/tmp` helps; if
they still fail, re-run them outside the restricted runner rather than
weakening the test.

## pnpm

### pnpm refuses to run: "modules purge" / non-TTY

Some pnpm operations (notably a workspace build after a lockfile change) prompt
when they want to purge `node_modules`, and the prompt cannot be answered in a
non-TTY context (agents, CI-like shells). Prefix with `CI=true`:

```bash
CI=true pnpm extension:build
```

### The install hook blocks you

New package installs must go through Socket Firewall (`sfw`), enforced by a
PreToolUse hook. Use `sfw pnpm install ...`, and see
[spec/package-install-security.md](spec/package-install-security.md). The
7-day `minimumReleaseAge` cooldown may resolve an older version than you
expect; that is deliberate.

## Git and signing

### Commit fails: "agent refused operation" / push or fetch fails auth

1Password's SSH agent is locked or not running. It handles both commit signing
and SSH remotes, so the same lock breaks commits, pushes, and `git fetch`
(which can also surface as a review runner failing to resolve `origin/main`).

Unlock 1Password. If you must proceed without it, use a **per-invocation**
override and say so:

```bash
git -c commit.gpgsign=false commit -m "..."
```

Never reconfigure global git or ssh to work around it.

### A review runner aborts with "worktree changed"

The review harnesses read the tree read-only. If you commit while one is
running, it aborts. Let reviews finish before mutating the worktree.
