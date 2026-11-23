# Contributing to Clovy

Thanks for helping build private AI.

## Before you start

- [AGENTS.md](AGENTS.md) is the canonical contributor guide. Read it first;
  this page is the short version and defers to it.
- [CONTEXT.md](CONTEXT.md) is the domain glossary. Using the right names
  (Clovy, CLI chat engine, dictation vs note transcription) keeps reviews fast.
- [docs/index.md](docs/index.md) maps every architecture doc, ADR, and
  runbook.

## Ways to contribute

- **Report a bug.** Open a GitHub issue.
- **Report a security vulnerability.** Never through a public issue; follow
  [SECURITY.md](SECURITY.md).
- **Propose a feature.** Open a feature request issue.
- **Send a PR.** For anything beyond a small fix, open an issue first so the
  approach is agreed before you build it.

## Development setup

Follow [docs/development.md](docs/development.md). The short version:

```sh
cp .env.example .env
sfw pnpm install
pnpm tauri:dev
```

The app has no server account or bearer token. Install and sign in to a CLI
for chat; save your provider keys in Settings for audio, remote text and media.
See [docs/configuration.md](docs/configuration.md).

## Before you open a PR

Run the checks CI runs:

```sh
make verify
```

Or individually: `pnpm check`, `pnpm typecheck`, `pnpm test`,
`pnpm test:rust`.

Frontend typecheck/tests and `src-tauri/` macOS Rust checks are local by default
on PRs to avoid paying for repeated hosted runners while a branch is still
changing. After pushing a clean branch, run the path-aware local signoff:

```sh
make local-ci
```

See [docs/local-ci-signoff.md](docs/local-ci-signoff.md) for setup, the
`run-frontend-ci` / `run-macos-ci` escape hatches, and maintainer ruleset notes.

Rules that fail review if violated (full list in [spec/index.md](spec/index.md)):

- Sentence case for all UI labels.
- No em or en dashes in user-facing copy.
- Icons from `central-icons` only.
- Use the design tokens in `src/styles/tokens.css`, not hand-coded values.

PR conventions, in brief (details in [AGENTS.md](AGENTS.md)):

- Fill in the PR template: what changed, validation, root cause for bug
  fixes, out of scope, followups.
- Attach a screenshot or recording for UI changes.
- Describe behavior generically; avoid naming or comparing other products in
  PR titles, descriptions, and release notes.
- State any provider-contract or user-configuration requirements.

## License

Clovy is MIT licensed. By contributing, you agree that your contributions are
licensed under the [MIT License](LICENSE).
