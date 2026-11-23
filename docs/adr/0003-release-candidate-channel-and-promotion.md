# Release-candidate channel and promote-to-stable

June ships a second update channel, `rc`, alongside `stable` (ADR 0001). An RC is
built on demand at a prerelease version, published to a fixed `rc` release in
`open-software-network/os-june-releases`, and tested through the in-app updater.
When it is good it is **promoted** to stable by rebuilding the same source at a
clean version. There is no direct stable-build path: every stable release starts
life as an RC.

## Status

accepted - implemented in PR #529

## Context

ADR 0001 established a single stable auto-update track and, critically, that the
Tauri updater's manifest endpoint is selectable **only in Rust** (the JS
`check()` cannot override `endpoints`). We want to ship test builds to opt-in
users before stable without a second app, without weakening signature
verification, and without a manual-install step for testers.

Two forces shape the whole design:

- **Channel selection must live in Rust.** Since the endpoint is Rust-only, the
  update check and install both move into Rust commands (`fetch_update` /
  `install_update` in `src-tauri/src/updates.rs`); the live `Update` handle
  cannot cross IPC. The channel is a persisted setting (`release-settings.json`)
  read Rust-side, so a check can never disagree with the saved channel.
- **The updater's `version_comparator` closure is the sole install gate.** In
  `tauri-plugin-updater` there is no separate downgrade block and no
  `allow_downgrades` on the Rust builder; returning `true` for an older remote
  installs a downgrade. Every install-decision rule therefore has to be expressed
  through that one closure.

## Decision

### Channels

`stable` follows `releases/latest/download/latest.json` (GitHub's `/latest`
redirect skips prereleases). `rc` follows a **fixed** `rc` release tag whose
`latest-rc.json` asset is overwritten each build, so the channel URL never moves.
Both inherit the same Ed25519 updater pubkey from `tauri.conf.json`, so signature
verification holds on both channels.

### Versioning (Q1-Q3)

- An RC is `X.Y.Z-rc.N` (semver prerelease). This orders `rc.1 < rc.2 < ... <
  X.Y.Z`, so a later candidate supersedes an earlier one and the eventual stable
  base supersedes them all.
- `main`'s version files bump **at promote, not at RC build**. RC versions are
  ephemeral (stamped into the build, never committed).
- Promote's clean version is the RC base with the `-rc.N` suffix dropped.

### Promote rebuilds from the recorded source (flavor B)

The RC records its exact source commit in an `rc-build.json` asset. Promote checks
out **that commit**, stamps the clean `X.Y.Z`, and reruns the full sign + notarize
path. Stable therefore ships the same source that was tested as the RC, with a
clean version string.

### Reconcile when leaving rc (Q4-Q8)

Switching the channel to `stable` while running a prerelease build offers a
**one-time, narrow downgrade** onto the current stable, because stable is normally
older than the RC you are on and a routine check would never pull it. The rule is
a pure function (`should_update`) wired into the `version_comparator`:

```
install if  remote > current                       (default, all routine checks)
        or  channel == Stable && reconcile && current.is_prerelease()
```

The escape is gated on `channel == Stable` and on a `reconcile` flag the frontend
sets **only** on the switch action, never on launch/periodic/manual checks. The
UI saves the channel immediately, then (if the running build is `-rc`) fetches the
stable version and shows a bespoke in-context confirm; the download reuses the
normal update flow, re-checking so a periodic check cannot leave a stale staged
update.

### Workflow gates (Q9, Q10)

- `rc-desktop-dmg.yml` rejects a `base-version` that is not strictly greater than
  `main`'s current version, so an RC can only ever target an unreleased version.
- `rc-desktop-dmg.yml` also rejects a composed `X.Y.Z-rc.N` that does not strictly
  exceed the version the `rc` release currently holds (allowing the first
  candidate when no `rc` release exists yet). The base-vs-`main` check alone
  ignores the rc iteration, so without this a rerun with an equal or lower
  `rc-number` would publish backward over the fixed channel URL and strand testers
  already on the higher build.
- `promote-desktop.yml` requires `rc-version` and fails unless it matches the RC
  release exactly, so a promote can never silently ship a different candidate.

### Windows builds from the promoted commit

`promote-desktop.yml` records the source commit in a `stable-build.json` asset on
the `vX.Y.Z` release. `production-desktop-windows.yml` reads that commit, checks
**it** out (not `main`), and stamps the clean version before building. macOS and
Windows therefore ship the same tested tree under one `vX.Y.Z`, even if `main`
advanced during the RC cycle, and the Windows build no longer depends on the
version-bump PR being merged first.

## Considered options

- **Promote by re-publishing the RC bytes as stable (flavor A)** (rejected). Would
  avoid a second build + notarize, but ships a binary whose internal version
  string is `X.Y.Z-rc.N`. Chose flavor B (rebuild clean) so the stable binary
  carries a clean version; accepted cost is one extra signed build.
- **Reconcile as a general stable rollback lever** (rejected). Chose the narrow
  prerelease-only escape (Q5): a bad stable release is fixed by rolling *forward*
  (`X.Y.Z+1`), never by installing an older stable. This keeps `stable -> stable`
  strictly forward-only and avoids a downgrade footgun.
- **Setting the escape comparator on the rc channel** (rejected). On `rc` the
  installed build is always a prerelease, so an unguarded escape would let `rc.2`
  "update" down to `rc.1` and destroy the Q1 ordering. The `channel == Stable`
  guard is load-bearing and is covered by a unit test.
- **P1 fix: gate promote to `BUILD_COMMIT == origin/main`** (rejected). Simpler,
  but forbids promoting a tested RC once any other PR merges to `main`. Chose to
  **decouple Windows from `main`** instead so `main` can advance freely during an
  RC cycle while both platforms still ship the RC commit.

## Consequences

- **Every channel is forward-only, enforced at three independent layers.** The
  updater's `should_update` gate (`remote > current`, except the guarded stable
  escape), the RC build's version-advance gate (composed `X.Y.Z-rc.N` must exceed
  the current `rc` release), and Q9 (`base-version > main`) each stop a different
  way the channel could move backward. No single layer covers all three; a build
  can pass Q9 yet still regress the `rc` channel, which is why the version-advance
  gate is separate.
- **No general stable rollback.** By design (Q5) there is no lever to push an
  older stable to users; recover from a bad stable by releasing forward.
- **Promote does a second full macOS build + notarize.** Slower than re-tagging,
  in exchange for a clean stable version string from tested source.
- **The version bump is bookkeeping, committed straight to `main`.** The release
  bot is on `main`'s branch-protection bypass list, so promote commits
  `release: vX.Y.Z` directly (no PR to merge), advancing the version files so the
  next RC's Q9 gate and the next changelog anchor work. It runs only after the
  stable release is published, so a build failure never leaves `main` bumped
  without a matching release. If the push loses a race with a concurrent commit
  to `main`, promote re-fetches and replays the deterministic bump onto the new
  tip (a few attempts) rather than failing after the release is already published
  and forcing a full rebuild rerun.
- **Release announcements are delegated to the GitHub Slack app.** Rather than a
  webhook step in the workflows, a channel subscribes with
  `/github subscribe open-software-network/os-june-releases releases`. This
  covers stable (each is a freshly published `vX.Y.Z`); RC iterations reuse the
  fixed `rc` tag and are edited in place, so they do not reliably post.
- **Post-RC commits merged during an RC cycle are not in that release.** They ship
  in a later release built from a future RC. Because the changelog anchors on the
  latest `release: v...` first-parent commit, such commits can fall between two
  changelogs; treat the changelog as a convenience, not a complete audit.
- **`stable-build.json` / `rc-build.json` are load-bearing release metadata.** A
  promote or Windows run reads them to learn the source commit; hand-editing or
  deleting them breaks the rebuild.
- **The escape rule must stay a tested pure function.** Its correctness is not
  observable from the updater builder, so `should_update` is unit-tested for the
  stable-escape, rc-forward-only, and clean-stable-forward-only cases.

## 2026-07-16 addendum: RC Slack announcements

Stable announcements remain delegated to the GitHub Slack app because every
stable version is a newly published `vX.Y.Z` release. RC iterations still edit
the fixed `rc` prerelease in place, so `rc-desktop-dmg.yml` now posts a
best-effort incoming-webhook message after the candidate assets are published.
The separate path is intentional: it announces every RC iteration without
changing the fixed-tag RC release channel contract. A missing or failing webhook
warns but does not fail the release after signed artifacts have published. The
fixed DMG asset continues to follow the RC release channel, while Slack links an
additional versioned DMG that remains immutable for the candidate named in the
announcement. The versioned asset is uploaded without replacement, and the RC
version guard fails closed unless GitHub explicitly reports that the fixed
release does not exist. Webhook delivery gets one bounded attempt because the
incoming webhook is not idempotent; on an unconfirmed result, the operator checks
Slack before manually posting a recovery announcement.

Superada pela change desvincular-repo-ci.
## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o atualizador nem a escolha de canal estável/RC; não há canal de atualização no app (2026-10-21).
