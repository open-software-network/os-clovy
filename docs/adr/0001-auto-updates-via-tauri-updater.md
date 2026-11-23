# Auto-updates via Tauri updater, hosted on a separate public releases repo

June ships in-app auto-updates using `tauri-plugin-updater` (single stable
track). Because the source repo is private and the Tauri updater fetches its
manifest over an **unauthenticated** GET, the signed + notarized artifacts and
`latest.json` are published to a **separate public repo**
(`open-software-network/os-june-releases`) rather than the private source
repo's releases. When the source repo eventually goes public, the updater
`endpoints` URL repoints to it and the releases repo is retired.

## Status

accepted

## Considered options

- **GitHub Releases on the private source repo** — rejected. Private release
  assets (including `latest.json`) return 404 to the updater's unauthenticated
  GET. The only workaround is embedding a repo-read token in the binary, which
  hands clone access to the private source to anyone holding the `.app`.
- **Railway storage bucket** — rejected. Railway buckets are private-only ("public
  buckets are currently not supported"); presigned URLs expire and cannot be
  baked into the app as a permanent endpoint, so the bucket would need a separate
  public proxy service in front — *more* infra than the alternative, for a
  temporary need. June API runs in a Phala TEE and is the wrong place to serve
  static files.
- **Cloudflare R2 public bucket** — viable (native public URLs, optional custom
  domain). Kept as the upgrade path if a permanent, GitHub-independent update
  domain is later wanted.
- **Separate public releases repo** — chosen. Zero new infra, reuses
  `gh release create` / `tauri-action`, and is a one-line endpoint change away
  from the eventual public-source-repo end state.

## Consequences

- The updater `endpoints` URL is **baked into every shipped build and is
  permanent for that build**. The chosen host must outlive the last install
  pointing at it; repointing requires a new release, and old installs keep
  polling the old URL until they update.
- Updater integrity uses a **separate Ed25519 signing key** (distinct from the
  Apple Developer ID cert). Losing its private key permanently breaks
  auto-update for all installs and forces a manual reinstall — back it up.
- Builds are **`aarch64`-only** for now; Intel Macs find no matching `platforms`
  entry in `latest.json` and silently will not auto-update.
- Builds that predate the updater cannot auto-update; the first updater-capable
  build must be installed manually once, automatic for every release thereafter.

## Update (2026-07-01)

Builds are now **universal** (x86_64 + arm64), so the `aarch64`-only consequence
above no longer applies: Intel Macs receive a matching `platforms` entry and
auto-update. The rc/stable release-channel switching is recorded in
[ADR-0003](0003-release-candidate-channel-and-promotion.md).

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o atualizador da June: o plugin updater, `updates.rs`, a UI e o item de menu "Check for Updates" foram removidos (2026-10-21).

## Addendum (2026-10-22): the fork's own updater

The updater returns for the owner's own releases, on `guilhermexp/os-clovy`
itself, with one stable channel and the owner's own signing key. See
[ADR-0069](0069-the-owner-publishes-own-releases-with-a-one-channel-updater.md).
