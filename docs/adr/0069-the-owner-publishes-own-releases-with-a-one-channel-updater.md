---
status: accepted
date: 2026-10-22
amends: 0001
---

# The owner publishes own releases with a one-channel updater

## Context

The fork removed the June updater and its release feed (ADR-0001 addendum,
change `desvincular-servicos-open-software`). The owner runs Clovy on a Mac and
on Windows and wants a release that updates itself, without depending on any
Open Software service. `guilhermexp/os-clovy` is public, so its releases answer
the updater's unauthenticated GET.

## Decision

- **Releases live on `guilhermexp/os-clovy` itself.** The only endpoint is
  `https://github.com/guilhermexp/os-clovy/releases/latest/download/latest.json`,
  written once in `plugins.updater.endpoints` (`tauri.conf.json`); no code names a
  URL. A separate releases repository (ADR-0001) only made sense for a private
  source repository.
- **One stable channel.** No release candidate channel, no channel setting, no
  `release-settings.json`. `/latest` resolves to the newest non-prerelease.
- **The owner's own minisign key.** The public key sits in
  `plugins.updater.pubkey`; the private key and its password live in the
  repository secrets `TAURI_SIGNING_PRIVATE_KEY` /
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` and in `~/.config/clovy/updater.key{,.password}`
  (mode 0600). Losing the key means every install must be reinstalled by hand.
- **Updater artifacts only in the release workflow.** `bundle.createUpdaterArtifacts`
  stays off in `tauri.conf.json`; `.github/workflows/release.yml` adds
  `src-tauri/tauri.release.conf.json`. Local builds need no private key.
- **Only a release build checks.** `updates::updates_enabled` is
  `!cfg!(debug_assertions)`; a development build never fetches the manifest and
  Settings shows no update action, so a dev bundle is never replaced by a release.
- **The owner approves every install.** A check (at launch, silent on failure,
  or from Settings > General) only offers the version. Nothing downloads until
  the owner accepts; "Later" hides the offer until the next launch or manual
  check. Install (which exits the app on Windows) and the restart wait while a
  meeting recording or dictation runs. The restart goes through the shutdown
  coordinator (`relaunch_for_update`).
- **Mac signing is optional.** Without Apple secrets the Mac build is signed ad
  hoc and not notarized; with them `scripts/build-macos-release.sh` signs with the
  identity and notarizes, with no code change. Windows stays without Authenticode.

## Consequences

- With ad hoc signing, macOS asks again for microphone, screen recording,
  accessibility and system audio after each update, and the first downloaded
  `.dmg` needs "Open Anyway" in Privacy & Security. Computer use does not run
  either: its driver authenticates the host by Apple team. All three go away
  once the Apple secrets exist.
- A tag `vX.Y.Z` that disagrees with the four version files fails before any
  build; a failed platform build publishes nothing.
- Rolling back is deleting the bad release: `/latest` falls back to the previous
  one; installs that already updated reinstall by hand.
