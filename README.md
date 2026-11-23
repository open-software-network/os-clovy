<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="public/clovy-wordmark.svg">
    <img src="public/clovy-wordmark.svg" alt="Clovy" width="160">
  </picture>
</p>

<h3 align="center">Private AI on your Mac</h3>

<p align="center">
  Clovy brings chat, voice dictation, meeting notes, and a local agent into a single
  private workspace. Local by default, with direct calls to the providers you
  configure, and open source so the privacy boundaries can be inspected.
</p>

<p align="center">
  <a href="LICENSE">
    <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue">
  </a>
</p>

## Why Clovy

Most AI apps ask you to hand over your most sensitive data and trust them with
it. Every prompt, file, and meeting reveals something about you, and a cloud
agent with that reach is a remote company's window into your work.

Clovy is built the other way around. The app and the agent run on your Mac.
Notes, recordings, transcripts, files, sessions, and agent memory stay on your
machine by default. Audio uses Groq or the optional local transcription engine; remote note text goes to
OpenRouter, and images/video to fal, each with your own key. Chat runs through
your installed CLI and its account. A configured endpoint can also generate
notes. What a provider retains depends on that provider and route.
You do not have to take any of this on faith: Clovy is MIT licensed; bundled dependencies and optional models retain their own licenses.

## Local transcription

On macOS 14 or later with Apple Silicon, open **Settings > Models > Transcription**,
click **Install model**, and choose **Local (Parakeet Ultra)** once it is ready.
The download is 632 MB, shows byte progress, and can be cancelled and resumed.
The model remains installed across restarts. Local handles dictation, recordings,
turn packs and live preview without a Groq key or transcription network requests.
A local failure stays local; it never sends your recording to Groq as a fallback.
Windows and Intel Macs keep Groq only.

Switching to Groq releases the local helper's memory. Removing the active model
switches transcription to Groq and shows a warning. Optional text cleanup and
note generation have separate provider settings and may still send text to them.
See [the audio pipeline](docs/audio-pipeline.md#local-transcription) for installation,
licenses, resource use, and the development self-test.

## What Clovy does

- **Chat.** Ask questions, do research, brainstorm, and build plans.
- **Dictation.** Hold a key, talk, release. Clovy turns your voice into clean,
  polished writing and pastes it into whatever app you were using, with
  push-to-talk and hands-free modes and selectable writing styles.
- **Meeting notes.** Clovy detects supported meetings and offers to take notes,
  without a bot joining the call. It records microphone or microphone plus
  system audio, orders the transcript into conversation turns, and generates
  editable notes. Saved audio is kept so failed steps can be retried without
  recording again.
- **Agent.** Claude Code runs chat through your installed CLI and its login,
  with Codex as the existing local fallback. Clovy supplies notes, skills,
  and other local tools through MCP. Sessions are sandboxed by default, and
  Clovy tool mutations wait for the local approval broker.
- **Image generation.** Generate and edit images, and create videos, directly
  through fal with your own key. Clovy's MCP media tools retain their local
  approval and artifact handling.
- **Your choice of models.** The text picker lists OpenRouter's text-output
  catalog, including models without tools. It reloads when the key changes,
  identifies the current Notes engine, and never changes the chat engine.
  Audio and media use curated model lists. Add provider keys in
  Settings > Models > More options > Provider API keys.

<table>
  <tr>
    <td width="50%">
      <img src=".github/assets/clovy-meeting-notes.jpg" alt="A meeting note in Clovy with a live transcription preview while recording">
    </td>
    <td width="50%">
      <img src=".github/assets/clovy-agent-analysis.jpg" alt="A Clovy agent session in private mode, reporting its analysis of a spreadsheet">
    </td>
  </tr>
  <tr>
    <td align="center">Meeting notes with live transcription, no bot in the call</td>
    <td align="center">The agent working through a spreadsheet in private mode</td>
  </tr>
</table>

## How Clovy keeps it private

1. **Local by default.** App state, recordings, transcripts, and agent memory
   live on your machine. CLI processes run locally with the selected execution
   policy; their model requests follow the CLI's own account and provider.
2. **Provider-dependent model privacy.** Audio, text, and media leave the
   device for the provider you selected. CLI model calls follow the CLI's
   account. There is no claim of zero retention without provider evidence.
3. **No Clovy account or billing.** There is no sign-in, local server token,
   credit balance, or shared service key. Provider usage belongs to your
   account. A missing key produces a setup message before a request.
4. **Inspectable boundaries.** The desktop and its local tools are built from
   this repository. Keys stay in the credential store and are sent only to
   their own provider; media downloads carry no credentials.

## Platforms

Clovy runs on macOS 14 or later, Apple Silicon and Intel. There are no
published releases: build it from source (see below).

Windows builds cover the app shell, microphone recording, notes, and
installed CLI chat engines. On Windows 10 1903+ and Windows 11 x64 they also
include encrypted text-only [activity capture](docs/activity-capture.md), the
[activity timeline](docs/activity-timeline.md), local coding-agent session
ingestion, and CPU embeddings for day intelligence. Global dictation paste,
system audio capture, and the macOS sandbox remain macOS-only. macOS is the
primary target.

## Repository layout

This repo contains the desktop app and its local tools.

```text
src/         React and TypeScript frontend
src-tauri/   Tauri v2 Rust desktop backend and native helpers
docs/        Architecture notes, ADRs, subsystem guides, and runbooks
spec/        Enforceable coding rules
specs/       Feature specs, plans, and validation notes
```

Provider keys are saved from Settings in the system credential store (Keychain
on macOS), never in the desktop `.env`. Start with [docs/index.md](docs/index.md),
[CONTEXT.md](CONTEXT.md), and [AGENTS.md](AGENTS.md).

## Build from source

You need Node.js with pnpm 11 and a Rust toolchain. The exact pnpm version is
pinned in `package.json`.

Windows native builds also need Visual Studio C++ Build Tools, Strawberry Perl,
and NASM on `PATH` for vendored OpenSSL and SQLCipher. The Windows build script
[`scripts/build-windows.ps1`](scripts/build-windows.ps1) checks Perl and NASM
before Cargo starts. Activity data uses the system credential store and remains
encrypted on both supported platforms.

```sh
git clone https://github.com/guilhermexp/os-clovy
cd os-clovy
cp .env.example .env
sfw pnpm install
pnpm tauri:dev
```

Install and sign in to Claude Code for chat; Codex remains the existing local
fallback. Save your Groq key for transcription and dictation, an OpenRouter
key for remote note generation and optional dictation cleanup, and a fal key
for media. No local inference server needs to be started. Sharing links and
the About verification action have been removed.

See [docs/development.md](docs/development.md) for the day-to-day development
guide (ports, onboarding replay, local data, permissions, agent skills, and
test commands) and [docs/configuration.md](docs/configuration.md) for the full
configuration reference, including exposing your own models.

## Contributing

Fork integration history: [versioned records](.upstream-sync/history/).

Clovy is also the canonical technical name for repository-controlled packages,
crates, environment variables, workflows, and release artifacts. Released
June-era inputs and rollback outputs remain compatibility aliases, while the
installed bundle, executable, updater, and permission identities stay stable.
See [ADR-0055](docs/adr/0055-clovy-technical-identity-migrates-through-a-compatibility-bridge.md).

```sh
pnpm check         # lint and format (Biome)
pnpm typecheck
pnpm test          # frontend (Vitest)
pnpm test:rust     # desktop Rust
```

`make verify` mirrors CI. Start with [CONTRIBUTING.md](CONTRIBUTING.md), then
[AGENTS.md](AGENTS.md) for the full contributor guide; the enforceable UI
rules live in [spec/](spec/index.md). Report bugs through GitHub issues, and
report security vulnerabilities privately per [SECURITY.md](SECURITY.md).

## License

Clovy is MIT licensed. See [LICENSE](LICENSE). Bundled third-party runtime
notices are tracked in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
